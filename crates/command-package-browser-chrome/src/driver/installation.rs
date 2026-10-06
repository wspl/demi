//! The pinned Chrome for Testing release, and on Linux the Chrome runtime it
//! starts with (`browser.md` § Browser distribution): `install` asks the
//! runner to install each archive from its URL over the artifacts stream
//! (`native-runtime.md` § The artifacts stream), following how the
//! downloads go, and every other command starts Chrome only from an
//! installation the runner already holds. Nothing here downloads anything
//! itself.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use demi_command_package_browser_protocol::browser::InstallResult;
use demi_command_package_browser_protocol::release::{
    ARTIFACT, BrowserRelease, ChromeRuntime, RUNTIME_FONTS, RUNTIME_FONTS_ENTRY,
    RUNTIME_LIBRARIES, RUNTIME_LIBRARIES_ENTRY, ReleasePlatform, RuntimeArchive, RuntimeArchives,
};
use demi_command_protocol::{ArtifactForm, ArtifactInstall, ArtifactProgress, host_target};
use demi_command_sdk::Artifacts;
use tokio::sync::{mpsc, watch};

use crate::driver::glibc::{self, GlibcVersion};
use crate::driver::launch::Runtime;
use crate::driver::operation::{BrowserError, Result};
use crate::driver::text;

/// Reads the Host's glibc version, none on a Host without glibc.
type GlibcReader = dyn Fn() -> std::io::Result<Option<GlibcVersion>> + Send + Sync;

/// The Chrome a service starts, through the runner that installs it.
/// Cloning shares the one source.
#[derive(Clone)]
pub struct Chrome {
    artifacts: Arc<watch::Sender<Option<Artifacts>>>,
    /// The target whose archives the Host takes: this program's own.
    target: &'static str,
    glibc: Arc<GlibcReader>,
}

impl Default for Chrome {
    fn default() -> Self {
        Self {
            artifacts: Arc::new(watch::Sender::new(None)),
            target: host_target(),
            glibc: Arc::new(glibc::host),
        }
    }
}

/// An installed Chrome to start: its executable, and on Linux the Chrome
/// runtime it starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installation {
    pub executable: PathBuf,
    pub runtime: Option<Runtime>,
}

impl Chrome {
    /// A Chrome on a Host of `target` whose glibc is `glibc`, whatever this
    /// machine is.
    #[cfg(any(test, feature = "testing"))]
    pub fn on(target: &'static str, glibc: Option<GlibcVersion>) -> Self {
        Self {
            target,
            glibc: Arc::new(move || Ok(glibc)),
            ..Self::default()
        }
    }

    /// Takes the service's artifacts source, which the runner answers.
    pub fn attach(&self, artifacts: Artifacts) {
        self.artifacts.send_replace(Some(artifacts));
    }

    /// Installs the pinned release for `invocation`, and on Linux the
    /// pinned runtime, unless the Host has them; a Linux Host with too old
    /// a glibc installs nothing. The lines that say how the downloads go
    /// reach `lines` (`browser.md` § Installation).
    pub async fn install(
        &self,
        invocation: &str,
        lines: mpsc::Sender<String>,
    ) -> Result<InstallResult> {
        let release = release()?;
        let platform = platform(&release, self.target)?;
        let runtime = release.runtime_archives(self.target);
        if runtime.is_some() {
            self.check_glibc().await?;
        }
        let source = self.source()?;
        let title = release.title();
        let chrome = ArtifactInstall {
            invocation: invocation.to_owned(),
            name: ARTIFACT.to_owned(),
            version: release.version.clone(),
            sha256: platform.sha256.clone(),
            size: platform.size,
            form: ArtifactForm::Archive {
                entry: platform.executable.clone(),
            },
            url: Some(platform.url.clone()),
        };
        let line = |progress| Some(text::install_progress(&title, progress));
        let path = install_reporting(&source, chrome, &lines, line)
            .await
            .map_err(|error| {
                BrowserError::Installation(format!("{title} could not be installed: {error}"))
            })?;
        if let Some(archives) = runtime {
            install_runtime(&source, invocation, &release.runtime, archives, &lines).await?;
        }
        Ok(InstallResult {
            browser: title,
            path: path.to_string_lossy().into_owned(),
        })
    }

    /// Fails unless the Host's glibc is one the runtime runs on, read on
    /// the blocking pool: the check reads the loader.
    async fn check_glibc(&self) -> Result<()> {
        let glibc = self.glibc.clone();
        let found = tokio::task::spawn_blocking(move || glibc()).await??;
        if found.is_some_and(|found| found >= glibc::OLDEST) {
            return Ok(());
        }
        let found = found.map_or_else(|| "none".to_owned(), |found| found.to_string());
        Err(BrowserError::Installation(format!(
            "Chrome on Linux needs glibc {} or newer; this Host has {found}",
            glibc::OLDEST
        )))
    }

    /// The pinned release to start, and on Linux the pinned runtime, when
    /// the Host has them installed (`browser.md` § Browser distribution).
    pub async fn installation(&self) -> Result<Installation> {
        let release = release()?;
        let platform = platform(&release, self.target)?;
        let archives = release.runtime_archives(self.target);
        let source = self.source()?;
        // Chrome and the runtime are downloaded together.
        let size = platform.size
            + archives.map_or(0, |archives| archives.libraries.size + archives.fonts.size);
        let missing = || BrowserError::Installation(text::not_installed(&release.title(), size));
        let executable = installed(&source, ARTIFACT, &platform.sha256)
            .await?
            .ok_or_else(missing)?;
        let runtime = match archives {
            Some(archives) => {
                let libraries = installed(&source, RUNTIME_LIBRARIES, &archives.libraries.sha256)
                    .await?
                    .ok_or_else(missing)?;
                let fonts = installed(&source, RUNTIME_FONTS, &archives.fonts.sha256)
                    .await?
                    .ok_or_else(missing)?;
                Some(Runtime::installed(&libraries, fonts))
            }
            None => None,
        };
        Ok(Installation {
            executable,
            runtime,
        })
    }

    /// Where Chrome's executables live, its helpers' included, for finding
    /// its processes: each installation of the line the runner has, once
    /// the service has its artifacts source; none when the runner could not
    /// list them, as when it ended the service's streams first.
    pub async fn roots(&self) -> Option<Vec<PathBuf>> {
        let mut attached = self.artifacts.subscribe();
        // The sender lives as long as this Chrome, so the wait ends only
        // with a source.
        let artifacts = attached.wait_for(Option::is_some).await.ok()?;
        let artifacts = artifacts.clone().expect("the wait ends with a source");
        match artifacts.installed(ARTIFACT).await {
            Ok(installed) => Some(
                installed
                    .iter()
                    .map(|installed| installation_of(Path::new(&installed.path)))
                    .collect(),
            ),
            Err(error) => {
                tracing::warn!("the installed Chrome could not be listed: {error}");
                None
            }
        }
    }

    /// The service's artifacts source, which a call always finds: the
    /// service takes it before it admits one.
    fn source(&self) -> Result<Artifacts> {
        self.artifacts.borrow().clone().ok_or_else(|| {
            BrowserError::Installation("demi-browser has no artifacts source".to_owned())
        })
    }
}

/// The path of the artifact of the line `name` whose digest is `sha256`,
/// when the runner holds it.
async fn installed(source: &Artifacts, name: &str, sha256: &str) -> Result<Option<PathBuf>> {
    let installed = source
        .installed(name)
        .await
        .map_err(|error| BrowserError::Installation(error.to_string()))?;
    Ok(installed
        .into_iter()
        .find(|installed| installed.sha256 == sha256)
        .map(|installed| PathBuf::from(installed.path)))
}

/// Installs `install`, sending to `lines` the line `line` makes of each
/// report of how it goes, when it makes one.
async fn install_reporting(
    source: &Artifacts,
    install: ArtifactInstall,
    lines: &mpsc::Sender<String>,
    mut line: impl FnMut(ArtifactProgress) -> Option<String>,
) -> std::result::Result<PathBuf, demi_command_sdk::ServiceError> {
    let (progress, mut reports) = Artifacts::progress();
    let installing = source.install(install, Some(progress));
    tokio::pin!(installing);
    let installed = loop {
        tokio::select! {
            biased;
            Some(report) = reports.recv() => send(lines, line(report)).await,
            installed = &mut installing => break installed,
        }
    };
    // What the runner reported before it answered.
    while let Ok(report) = reports.try_recv() {
        send(lines, line(report)).await;
    }
    installed
}

/// Sends `line`, if any, to `lines`. Their reader outlives every install,
/// as the invocation awaits the install; once it stops reading, the lines
/// have nowhere to go.
async fn send(lines: &mpsc::Sender<String>, line: Option<String>) {
    if let Some(line) = line {
        let _sent = lines.send(line).await;
    }
}

/// Installs the runtime's `archives` for `invocation`, each as an artifact
/// of its own line, and reports them as one download: a line at each tenth
/// of their total size, and one as the last is unpacked.
async fn install_runtime(
    source: &Artifacts,
    invocation: &str,
    runtime: &ChromeRuntime,
    archives: RuntimeArchives<'_>,
    lines: &mpsc::Sender<String>,
) -> Result<()> {
    let title = format!("the {}", runtime.title());
    let total = archives.libraries.size + archives.fonts.size;
    let parts = [
        (RUNTIME_LIBRARIES, archives.libraries, RUNTIME_LIBRARIES_ENTRY),
        (RUNTIME_FONTS, archives.fonts, RUNTIME_FONTS_ENTRY),
    ];
    let mut before = 0;
    let mut shown = 0;
    for (index, (name, archive, entry)) in parts.into_iter().enumerate() {
        let last = index + 1 == parts.len();
        let install = runtime_install(invocation, runtime, name, archive, entry);
        let line = |progress| match progress {
            ArtifactProgress::Download { done, .. } => {
                let done = before + done;
                let tenth = done.saturating_mul(10) / total;
                if tenth <= shown {
                    return None;
                }
                shown = tenth;
                let progress = ArtifactProgress::Download { done, total };
                Some(text::install_progress(&title, progress))
            }
            ArtifactProgress::Unpack => {
                last.then(|| text::install_progress(&title, ArtifactProgress::Unpack))
            }
        };
        install_reporting(source, install, lines, line)
            .await
            .map_err(|error| {
                BrowserError::Installation(format!(
                    "The {} could not be installed: {error}",
                    runtime.title()
                ))
            })?;
        before += archive.size;
    }
    Ok(())
}

/// The install of the runtime's `archive`, of the line `name`, whose entry
/// is `entry`.
fn runtime_install(
    invocation: &str,
    runtime: &ChromeRuntime,
    name: &str,
    archive: &RuntimeArchive,
    entry: &str,
) -> ArtifactInstall {
    ArtifactInstall {
        invocation: invocation.to_owned(),
        name: name.to_owned(),
        version: runtime.release.to_string(),
        sha256: archive.sha256.clone(),
        size: archive.size,
        form: ArtifactForm::Archive {
            entry: entry.to_owned(),
        },
        url: Some(archive.url.clone()),
    }
}

/// The directory that holds `executable` and its helpers: macOS helpers live
/// in the app's Frameworks directory, Linux helpers beside the main
/// executable.
pub(crate) fn installation_of(executable: &Path) -> PathBuf {
    executable
        .ancestors()
        .find(|path| path.extension().is_some_and(|extension| extension == "app"))
        .or_else(|| executable.parent())
        .expect("absolute Chrome executable has a parent")
        .to_owned()
}

/// The pinned Chrome for Testing release.
fn release() -> Result<BrowserRelease> {
    BrowserRelease::pinned().map_err(|error| BrowserError::Configuration(error.to_string()))
}

/// The release's archive for `target`.
fn platform<'a>(release: &'a BrowserRelease, target: &str) -> Result<&'a ReleasePlatform> {
    release.platform(target).ok_or_else(|| {
        BrowserError::Installation(format!("{} is unavailable on {target}", release.title()))
    })
}

/// The pinned release's version, such as `153.0.8010.36`.
pub fn pinned_version() -> Result<String> {
    Ok(release()?.version)
}
