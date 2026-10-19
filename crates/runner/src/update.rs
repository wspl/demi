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
    time::Duration,
};

use demi_command_protocol::host_target;
use demi_runner_protocol::{
    release::{COMPRESSED_SUFFIX, RELEASE_ENV, RUNNER, RunnerUpdate, TOKEN_HEADER},
    values::{BackendUrl, DeviceToken},
};
use demi_shared_artifacts::{Digest, Mode, Permissions, Publication};
use tokio_util::sync::CancellationToken;

/// How long a download of a runner executable waits for its next byte
/// before it gives that connection up and resumes on a new one
/// (`runner.md` § Runner updates).
pub const DOWNLOAD_STALL: Duration = demi_shared_artifacts::Patience::DEFAULT.stall;

/// An installation that an installer made, whose runner this process is:
/// its executable is `<root>/releases/<release>/demi-runner`.
pub struct Installed {
    root: PathBuf,
    release: String,
    /// How long a download waits for its next byte: [`DOWNLOAD_STALL`],
    /// shorter in a test.
    stall: Duration,
}

/// The runner that takes this one's place.
pub struct Successor {
    executable: PathBuf,
    release: String,
}

impl Installed {
    /// The installation at `root` when `executable` is its runner of
    /// `release`; none for a runner started otherwise, such as a managed
    /// guest's or one a developer runs from a build. Its downloads give a
    /// connection up after `stall` without a byte.
    pub fn of(
        root: &Path,
        release: Option<&str>,
        executable: &Path,
        stall: Duration,
    ) -> Option<Self> {
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
            stall,
        })
    }

    /// Puts the executable of `release`, whose decompressed copy `expected`
    /// describes, in place ahead of an update, downloaded from `backend`
    /// while this runner goes on serving (`runner.md` § Runner updates).
    pub async fn prefetch(
        &self,
        backend: &BackendUrl,
        token: Option<&DeviceToken>,
        release: &str,
        expected: &Digest,
        cancel: &CancellationToken,
    ) -> io::Result<()> {
        let _lock = InstallerLock::take(&self.root)?;
        self.place(backend, token, release, expected, cancel)
            .await
            .map(drop)
    }

    /// Puts the executable `update` names in place, downloaded from
    /// `backend` unless a download ahead put it there, and makes it the
    /// installation's release. Every release directory but the new one and
    /// this runner's goes.
    pub async fn update(
        &self,
        backend: &BackendUrl,
        token: Option<&DeviceToken>,
        update: &RunnerUpdate,
        cancel: &CancellationToken,
    ) -> io::Result<Successor> {
        let Some(executable) = &update.executable else {
            return Err(io::Error::other(format!(
                "the backend's runner release {} has no runner for {}",
                update.release,
                host_target()
            )));
        };
        let expected = Digest {
            size: executable.size,
            sha256: executable.sha256.clone(),
        };
        let _lock = InstallerLock::take(&self.root)?;
        let path = self
            .place(backend, token, &update.release, &expected, cancel)
            .await?;
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
        self.prune(&self.root.join("releases"), &update.release).await;
        Ok(Successor {
            executable: path,
            release: update.release.clone(),
        })
    }

    /// The executable of `release` in place in `releases/<release>/`,
    /// checked against `expected`: the one there, or one downloaded from
    /// `backend` as its compressed copy (`native-runtime.md` § Runner
    /// releases). The download keeps its bytes in
    /// `releases/.download-<release>/` across attempts and resumes from
    /// them; it is decompressed and checked once complete, and bytes that
    /// fail the check are removed. The caller holds the install lock.
    async fn place(
        &self,
        backend: &BackendUrl,
        token: Option<&DeviceToken>,
        release: &str,
        expected: &Digest,
        cancel: &CancellationToken,
    ) -> io::Result<PathBuf> {
        let releases = self.root.join("releases");
        let directory = releases.join(release);
        let name = file_name();
        let path = directory.join(&name);
        if directory.exists() {
            let found = demi_shared_artifacts::digest(&path, expected.size, cancel)
                .await
                .map_err(io::Error::other)?;
            if found != *expected {
                return Err(io::Error::other(format!(
                    "the installed runner of release {release} does not match its release"
                )));
            }
            return Ok(path);
        }
        let url = backend
            .url()
            .join(&format!(
                "/runner-artifacts/{release}/{}/{name}{COMPRESSED_SUFFIX}",
                host_target()
            ))
            .map_err(io::Error::other)?;
        let download = releases.join(format!(".download-{release}"));
        tokio::fs::create_dir_all(&download).await?;
        let compressed = download.join(format!("{name}{COMPRESSED_SUFFIX}"));
        // The digest came from the backend this runner trusts, as an
        // installer's does, so the transport cannot change what is kept; the
        // token lets the backend count the device's progress. Connections
        // that deliver nothing for twice the stall fail it, as the default
        // patience does.
        let client = demi_shared_artifacts::download_client_allowing_http().map_err(io::Error::other)?;
        let headers = |_: &reqwest::Url| {
            let mut headers = reqwest::header::HeaderMap::new();
            if let Some(value) = token.and_then(|token| {
                reqwest::header::HeaderValue::from_str(token.expose()).ok()
            }) {
                headers.insert(TOKEN_HEADER, value);
            }
            headers
        };
        let request = demi_shared_artifacts::Download {
            patience: demi_shared_artifacts::Patience {
                stall: self.stall,
                silence: self.stall * 2,
            },
            headers: &headers,
            ..demi_shared_artifacts::Download::new(url.as_str())
        };
        // The backend counts the bytes it serves to the device itself, and
        // serves the compressed file without a content coding, which the
        // check below decodes.
        demi_shared_artifacts::download(&client, request, &compressed, cancel, |_, _| {})
            .await
            .map_err(io::Error::other)?;
        let stage = download.join("release");
        match tokio::fs::remove_dir_all(&stage).await {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
        }
        tokio::fs::create_dir(&stage).await?;
        if let Err(error) = decode(&compressed, &stage.join(&name), expected, cancel).await {
            // Bytes that fail the check are no start for the next attempt;
            // a cancelled check says nothing of them.
            if !matches!(error, demi_shared_artifacts::Error::Cancelled) {
                tokio::fs::remove_file(&compressed).await?;
            }
            return Err(io::Error::other(error));
        }
        demi_shared_artifacts::publish_directory(&stage, &directory)
            .await
            .map_err(io::Error::other)?;
        // The release is in place; its download holds nothing anyone needs.
        if let Err(error) = tokio::fs::remove_dir_all(&download).await {
            tracing::warn!("the download {} stays: {error}", download.display());
        }
        Ok(path)
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

    /// Starts the successor beside this process, which then exits; with
    /// this process's arguments, it writes the log this one wrote. It is no
    /// runner the service manager started, whose task ends with this
    /// process, so it hands the installation over to the manager once it
    /// holds it (`runner.md` § Starting with the computer).
    #[cfg(windows)]
    pub fn start(self) -> io::Result<()> {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        self.command()
            .env_remove(demi_runner_protocol::SUPERVISED)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()?;
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

/// Decodes the downloaded compressed copy at `compressed` into a new
/// executable file at `path`, checking its size and SHA-256.
async fn decode(
    compressed: &Path,
    path: &Path,
    expected: &Digest,
    cancel: &CancellationToken,
) -> Result<(), demi_shared_artifacts::Error> {
    let mut staged = demi_shared_artifacts::Staged::new(
        path,
        Publication {
            mode: Mode::CreateNew,
            permissions: Permissions::Executable,
            durable: true,
        },
    )
    .await?;
    demi_shared_artifacts::decode_file(compressed, expected, staged.file(), cancel).await?;
    staged.publish().await
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
