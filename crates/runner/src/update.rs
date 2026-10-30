//! A paired device's runner replacing itself with the runner release of its
//! backend.
//!
//! It puts the backend's executable in the installation's `releases/` as the
//! installers do, under their lock, moves it over the installation's runner,
//! `<installation>/demi-runner`, names it in `release-id`, and starts it in its
//! own place once the registration has ended. The jobs run on in their shells'
//! processes meanwhile, and the new runner takes them over and lists them in its
//! hello (`crates/runner-shell/src/remote/mod.rs`).
//!
//! # One path
//!
//! Every release runs from `<installation>/demi-runner`, so the system tells
//! one program across updates: a Mac's privacy permissions follow the path of a
//! program outside an app bundle, and a new path asked for them all again.
//!
//! - On Unix the new executable is renamed over the running one, whose process
//!   keeps its own file.
//! - Windows cannot replace a running executable but can rename it: the running
//!   one moves to `releases/.replaced-<release>.exe`, which the next update
//!   removes.
//! - Every update writes the installation's launcher anew, so a launcher never
//!   names a file an update removed.
//! - An installation from before ran each release from
//!   `releases/<release>/demi-runner`. Its runner of this release links itself
//!   to the one path as it starts ([`Installed::settle`]), writes the
//!   installation's launcher anew, and starts from there. The old path stays
//!   until the launcher is written, so a failure leaves a launcher that still
//!   starts a runner, which tries again at its next start.
//!
//! # Downloading ahead
//!
//! The runner downloads ahead. While the old release still serves, an upgrade
//! tells the backend which release comes next, and the backend sends each
//! connected paired device's runner `prefetch`. The device stays online and,
//! once the server switches, finds the executable in place and is back within
//! seconds.
//!
//! Without it a device on a slow link (a server reaching the backend at 7 KB/s)
//! downloaded after the switch, showed offline for minutes and broke off its
//! first download.
//!
//! - A failed prefetch is harmless, since the update downloads what is missing.
//! - One release waits ahead at most: a server that got a newer release ready
//!   since has its devices download that one, and their download removes the
//!   earlier one's.
//! - Every download of a runner executable, ahead or in an update, is Demi's
//!   one download (`demi_shared_artifacts`), staged in
//!   `releases/.download-<release>/` and resumed.
//!
//! # Failures
//!
//! - A failed update leaves the runner on its release and is retried at its
//!   next connection attempt.
//! - A 409 without an executable (no runner for the target) is logged and
//!   retried, since a later release may have one.
//! - A managed guest's runner never updates itself: a 409 for it means the
//!   backend and the machine manager run different releases, so it exits naming
//!   both and the Cloud's boot fails.

use std::{
    ffi::OsString,
    io,
    path::{Path, PathBuf},
    process::Command,
};

use demi_command_protocol::host_target;
use demi_runner_protocol::{
    installation::{LAUNCHER_PS1, LAUNCHER_SH},
    release::{COMPRESSED_SUFFIX, RELEASE_ENV, RUNNER, RunnerUpdate, TOKEN_HEADER},
    values::{BackendUrl, DeviceToken},
};
use demi_shared_artifacts::{Digest, Mode, Permissions, Publication};
use tokio_util::sync::CancellationToken;

/// An installation that an installer made, whose runner this process is:
/// its executable is `<root>/demi-runner`, or in an installation from before
/// `<root>/releases/<release>/demi-runner`, until it settles.
pub struct Installed {
    root: PathBuf,
    release: String,
    /// This process's executable, canonical.
    running: PathBuf,
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
        let running = executable.canonicalize().ok()?;
        let runs = |path: PathBuf| path.canonicalize().is_ok_and(|path| path == running);
        let installed = runs(root.join(file_name()))
            || runs(root.join("releases").join(release).join(file_name()));
        installed.then(|| Self {
            root: root.to_owned(),
            release: release.to_owned(),
            running,
        })
    }

    /// The file every release of the installation runs from.
    fn executable(&self) -> PathBuf {
        self.root.join(file_name())
    }

    /// Links the runner of an installation from before, which runs from its
    /// release's directory, to the installation's one path, writes the
    /// installation's launcher anew, which starts it there, and removes the
    /// releases. The runner to start in this one's place; none when this one
    /// runs from the one path already.
    pub async fn settle(&self) -> io::Result<Option<Successor>> {
        let executable = self.executable();
        if executable.canonicalize().is_ok_and(|path| path == self.running) {
            return Ok(None);
        }
        let _lock = InstallerLock::take(&self.root)?;
        // An earlier start that failed to write the launcher leaves one,
        // which no process runs.
        remove_if_present(&executable).await?;
        tokio::fs::hard_link(&self.running, &executable).await?;
        if let Err(error) = self.publish_launcher().await {
            remove_if_present(&executable).await?;
            return Err(error);
        }
        self.prune().await;
        Ok(Some(Successor {
            executable,
            release: self.release.clone(),
        }))
    }

    /// Puts the executable of `release`, whose decompressed copy `expected`
    /// describes, in place ahead of an update, downloaded from `backend`
    /// while this runner goes on serving.
    pub async fn prefetch(
        &self,
        backend: &BackendUrl,
        token: Option<&DeviceToken>,
        release: &str,
        expected: &Digest,
        cancel: &CancellationToken,
    ) -> io::Result<()> {
        let _lock = InstallerLock::take(&self.root)?;
        self.place(backend, token, release, expected, cancel).await?;
        self.prune_ahead(release).await;
        Ok(())
    }

    /// Removes what an earlier download ahead left in `releases/`, the
    /// executable of a release that `release` came after and its download,
    /// so one release waits ahead at most. The running release's directory,
    /// which an installation from before runs from, and what the system holds
    /// open, such as the executable Windows renamed aside, stay.
    async fn prune_ahead(&self, release: &str) {
        let mut entries = match tokio::fs::read_dir(self.root.join("releases")).await {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!(event = "runner_update.prune_failed", %error, "an earlier runner release downloaded ahead stays");
                return;
            }
        };
        let download = format!(".download-{release}");
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name == release || name == self.release || name == download || name.starts_with(".replaced-") {
                continue;
            }
            let removed = match entry.file_type().await {
                Ok(kind) if kind.is_dir() => tokio::fs::remove_dir_all(entry.path()).await,
                _ => tokio::fs::remove_file(entry.path()).await,
            };
            if let Err(error) = removed {
                tracing::warn!(event = "runner_update.prune_failed", release = %name, %error, "an earlier runner release downloaded ahead stays");
            }
        }
    }

    /// Puts the executable `update` names in place, downloaded from
    /// `backend` unless a download ahead put it there, moves it over the
    /// installation's runner and makes it the installation's release. Every
    /// other release goes.
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
        let executable = self.executable();
        // Windows renames the running executable aside, which it cannot
        // replace; a runner that runs from its release's directory leaves
        // none there.
        #[cfg(windows)]
        if tokio::fs::try_exists(&executable).await? {
            let aside = self
                .root
                .join("releases")
                .join(format!(".replaced-{}.exe", self.release));
            remove_if_present(&aside).await?;
            tokio::fs::rename(&executable, &aside).await?;
        }
        tokio::fs::rename(&path, &executable).await?;
        self.publish_launcher().await?;
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
        self.prune().await;
        Ok(Successor {
            executable,
            release: update.release.clone(),
        })
    }

    /// Writes the installation's launcher, which starts the one path.
    async fn publish_launcher(&self) -> io::Result<()> {
        let (name, launcher) = if cfg!(windows) {
            ("run.ps1", LAUNCHER_PS1)
        } else {
            ("run", LAUNCHER_SH)
        };
        demi_shared_artifacts::publish_bytes(
            &self.root.join(name),
            launcher.as_bytes(),
            Publication {
                mode: Mode::Replace,
                permissions: Permissions::Executable,
                durable: true,
            },
        )
        .await
        .map_err(io::Error::other)
    }

    /// The executable of `release` in place in `releases/<release>/`, checked
    /// against `expected`: the one there, or one downloaded from `backend` as
    /// its compressed copy (`crates/backend-http/src/install.rs`). A directory
    /// whose executable is missing or differs, such as one an update that
    /// failed midway emptied, is downloaded again.
    ///
    /// The download keeps its bytes in `releases/.download-<release>/` across
    /// attempts and resumes from them. It is decompressed and checked once
    /// complete, and bytes that fail the check are removed. The caller holds
    /// the install lock.
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
        if path.exists() {
            let found = demi_shared_artifacts::digest(&path, expected.size, cancel)
                .await
                .map_err(io::Error::other)?;
            if found == *expected {
                return Ok(path);
            }
        }
        match tokio::fs::remove_dir_all(&directory).await {
            Err(error) if error.kind() != io::ErrorKind::NotFound => return Err(error),
            _ => {}
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
        // that deliver nothing for the default patience fail it.
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
            tracing::warn!(event = "runner_update.download_kept", file = %download.display(), %error, "a finished download stays");
        }
        Ok(path)
    }

    /// Removes every release in `releases/`, the one path holding the
    /// installation's runner; what the system still holds open, such as the
    /// running executable Windows renamed aside, goes at the next update.
    async fn prune(&self) {
        let mut entries = match tokio::fs::read_dir(self.root.join("releases")).await {
            Ok(entries) => entries,
            Err(error) => {
                tracing::warn!(event = "runner_update.prune_failed", %error, "the earlier runner releases stay");
                return;
            }
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            let name = entry.file_name();
            let removed = match entry.file_type().await {
                Ok(kind) if kind.is_dir() => tokio::fs::remove_dir_all(entry.path()).await,
                _ => tokio::fs::remove_file(entry.path()).await,
            };
            if let Err(error) = removed {
                tracing::warn!(event = "runner_update.prune_failed", release = %name.to_string_lossy(), %error, "an earlier runner release stays");
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
    /// holds it (`crates/runner/src/autostart.rs`).
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
async fn remove_if_present(path: &Path) -> io::Result<()> {
    match tokio::fs::remove_file(path).await {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

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
            tracing::warn!(event = "installer_lock.kept", file = %self.path.display(), %error, "the installer lock stays");
        }
    }
}
