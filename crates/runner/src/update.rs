//! A paired device's runner replacing itself with the runner release of its
//! backend (`runner.md` § Runner updates): it puts the backend's executable
//! in its installation's `releases/` as the installers do, under their lock,
//! names it in `release-id`, and starts it in its own place once the
//! registration has ended.

use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    process::Command,
};

use demi_command_protocol::host_target;
use demi_runner_protocol::{
    release::{RELEASE_ENV, RUNNER, RunnerUpdate},
    values::BackendUrl,
};
use demi_shared_artifacts::{Digest, Mode, Permissions, Publication};
use tokio_util::sync::CancellationToken;

/// An installation that an installer made, whose runner this process is:
/// its executable is `<root>/releases/<release>/demi-runner`.
pub struct Installed {
    root: PathBuf,
    release: String,
}

/// The runner that takes this one's place.
pub struct Successor {
    executable: PathBuf,
    release: String,
}

impl Installed {
    /// The installation at `root` when `executable` is its runner of
    /// `release`; none for a runner started otherwise, such as a managed
    /// guest's or one a developer runs from a build.
    pub fn of(root: &Path, release: Option<&str>, executable: &Path) -> Option<Self> {
        let release = release?;
        let installed = root
            .join("releases")
            .join(release)
            .join(executable.file_name()?)
            .canonicalize()
            .ok()?;
        let running = executable.canonicalize().ok()?;
        (installed == running).then(|| Self {
            root: root.to_owned(),
            release: release.to_owned(),
        })
    }

    /// Puts the executable `update` names in place, downloaded from
    /// `backend`, and makes it the installation's release. Every release
    /// directory but the new one and this runner's goes.
    pub async fn update(
        &self,
        backend: &BackendUrl,
        update: &RunnerUpdate,
        cancel: &CancellationToken,
    ) -> io::Result<Successor> {
        let target = host_target();
        let Some(executable) = &update.executable else {
            return Err(io::Error::other(format!(
                "the backend's runner release {} has no runner for {target}",
                update.release
            )));
        };
        let expected = Digest {
            size: executable.size,
            sha256: executable.sha256.clone(),
        };
        let _lock = InstallerLock::take(&self.root)?;
        let releases = self.root.join("releases");
        let directory = releases.join(&update.release);
        let name = file_name();
        let path = directory.join(&name);
        if directory.exists() {
            let found = demi_shared_artifacts::digest(&path, expected.size, cancel)
                .await
                .map_err(io::Error::other)?;
            if found != expected {
                return Err(io::Error::other(format!(
                    "the installed runner of release {} does not match its release",
                    update.release
                )));
            }
        } else {
            let url = backend
                .url()
                .join(&format!(
                    "/runner-artifacts/{}/{target}/{name}",
                    update.release
                ))
                .map_err(io::Error::other)?;
            let stage = releases.join(format!(".download-{}", uuid::Uuid::new_v4().simple()));
            let downloaded = download(url.as_str(), &stage.join(&name), &expected, cancel).await;
            let placed = match downloaded {
                Ok(()) => demi_shared_artifacts::publish_directory(&stage, &directory)
                    .await
                    .map_err(io::Error::other),
                Err(error) => Err(error),
            };
            if let Err(error) = placed {
                // The stage holds nothing anyone needs; the update's own
                // error is the one to report.
                let _ = tokio::fs::remove_dir_all(&stage).await;
                return Err(error);
            }
        }
        demi_shared_artifacts::publish_bytes(
            &self.root.join("release-id"),
            format!("{}\n", update.release).as_bytes(),
            Publication {
                mode: Mode::Replace,
                permissions: Permissions::Private,
                durable: true,
            },
        )
        .await
        .map_err(io::Error::other)?;
        self.prune(&releases, &update.release).await;
        Ok(Successor {
            executable: path,
            release: update.release.clone(),
        })
    }

    /// Removes every release directory but `kept`'s and this runner's.
    async fn prune(&self, releases: &Path, kept: &str) {
        let mut entries = match tokio::fs::read_dir(releases).await {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!("the earlier runner releases stay: {error}");
                return;
            }
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            if name == *kept || name == *self.release {
                continue;
            }
            if let Err(error) = tokio::fs::remove_dir_all(entry.path()).await {
                tracing::warn!("runner release {name:?} stays: {error}");
            }
        }
    }
}

impl Successor {
    /// Starts the successor with this process's arguments, in this process
    /// on Unix, where it returns only an error.
    #[cfg(unix)]
    pub fn start(self) -> io::Result<()> {
        use std::os::unix::process::CommandExt as _;
        Err(self.command().exec())
    }

    /// Starts the successor beside this process, which then exits; the
    /// successor shares its standard streams, as the installer redirected
    /// them.
    #[cfg(windows)]
    pub fn start(self) -> io::Result<()> {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        self.command().creation_flags(CREATE_NO_WINDOW).spawn()?;
        Ok(())
    }

    fn command(&self) -> Command {
        let mut command = Command::new(&self.executable);
        command
            .args(std::env::args_os().skip(1).collect::<Vec<OsString>>())
            .env(RELEASE_ENV, &self.release);
        command
    }
}

/// The runner executable's file name on this platform.
fn file_name() -> String {
    if cfg!(windows) {
        format!("{RUNNER}.exe")
    } else {
        RUNNER.to_owned()
    }
}

/// Downloads `url` into a new executable file at `path`, checking its size
/// and SHA-256.
async fn download(
    url: &str,
    path: &Path,
    expected: &Digest,
    cancel: &CancellationToken,
) -> io::Result<()> {
    tokio::fs::create_dir_all(path.parent().expect("the file lies in the stage")).await?;
    // The digest came from the backend this runner trusts, as an installer's
    // does, so the transport cannot change what is kept.
    let client = demi_shared_artifacts::client_allowing_http().map_err(io::Error::other)?;
    let mut staged = demi_shared_artifacts::Staged::new(
        path,
        Publication {
            mode: Mode::CreateNew,
            permissions: Permissions::Executable,
            durable: true,
        },
    )
    .await
    .map_err(io::Error::other)?;
    demi_shared_artifacts::download(&client, url, expected, staged.file(), cancel)
        .await
        .map_err(io::Error::other)?;
    staged.publish().await.map_err(io::Error::other)
}

/// The lock the installers take on an installation: a directory on Unix,
/// which the shell installer makes, and a file opened for no one else on
/// Windows, as the PowerShell installer opens it.
struct InstallerLock {
    #[cfg(unix)]
    path: PathBuf,
    #[cfg(windows)]
    _file: std::fs::File,
}

impl InstallerLock {
    fn take(root: &Path) -> io::Result<Self> {
        let path = root.join("install.lock");
        let busy = |error: io::Error| {
            io::Error::other(format!("an installer is working on this installation: {error}"))
        };
        #[cfg(unix)]
        {
            std::fs::create_dir(&path).map_err(busy)?;
            Ok(Self { path })
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt as _;
            let file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(false)
                .share_mode(0)
                .open(&path)
                .map_err(busy)?;
            Ok(Self { _file: file })
        }
    }
}

/// The shell installer removes its lock directory as it ends, and so does
/// this lock; the Windows installer leaves its file, whose closing releases
/// the lock.
#[cfg(unix)]
impl Drop for InstallerLock {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir(&self.path) {
            tracing::warn!("the installer lock {} stays: {error}", self.path.display());
        }
    }
}
