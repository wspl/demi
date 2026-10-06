//! The pinned Chrome for Testing release (`browser.md` § Browser
//! distribution): `install` asks the runner to install it from its official
//! URL over the artifacts stream (`native-runtime.md` § The artifacts
//! stream), following how the download goes, and every other command starts
//! it only from an installation the runner already holds, on a Host that has
//! the libraries it loads. Nothing here downloads Chrome itself.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use demi_command_package_browser_protocol::browser::InstallResult;
use demi_command_package_browser_protocol::release::{ARTIFACT, BrowserRelease, ReleasePlatform};
use demi_command_protocol::{ArtifactForm, ArtifactInstall, ArtifactProgress, host_target};
use demi_command_sdk::Artifacts;
use tokio::sync::{mpsc, watch};

use crate::driver::operation::{BrowserError, Result};
use crate::driver::requirements::{self, Missing};
use crate::driver::text;

/// What a Host lacks for the Chrome at an executable, which is an entry in
/// its archive.
type Requirements = dyn Fn(&Path, &str) -> Result<Missing> + Send + Sync;

/// The Chrome a service starts, through the runner that installs it.
/// Cloning shares the one source.
#[derive(Clone)]
pub struct Chrome {
    artifacts: Arc<watch::Sender<Option<Artifacts>>>,
    requirements: Arc<Requirements>,
}

impl Default for Chrome {
    fn default() -> Self {
        Self {
            artifacts: Arc::new(watch::Sender::new(None)),
            requirements: Arc::new(requirements::missing),
        }
    }
}

impl Chrome {
    /// A Chrome on a Host that lacks `missing` for it, whatever the Host's
    /// own files say.
    #[cfg(any(test, feature = "testing"))]
    pub fn lacking(missing: Missing) -> Self {
        Self {
            requirements: Arc::new(move |_: &Path, _: &str| Ok(missing.clone())),
            ..Self::default()
        }
    }

    /// Takes the service's artifacts source, which the runner answers.
    pub fn attach(&self, artifacts: Artifacts) {
        self.artifacts.send_replace(Some(artifacts));
    }

    /// Installs the pinned release for `invocation` from its official URL,
    /// unless the Host has it, and names what the Host still lacks for it.
    /// How its download goes reaches `progress`.
    pub async fn install(
        &self,
        invocation: &str,
        progress: mpsc::Sender<ArtifactProgress>,
    ) -> Result<InstallResult> {
        let release = release()?;
        let platform = platform(&release)?;
        let install = ArtifactInstall {
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
        let path = self.source()?.install(install, Some(progress)).await.map_err(|error| {
            BrowserError::Installation(format!("{} could not be installed: {error}", release.title()))
        })?;
        let missing = self.missing(&path, &platform.executable).await?;
        Ok(InstallResult {
            browser: release.title(),
            path: path.to_string_lossy().into_owned(),
            missing_libraries: missing.libraries,
            missing_fonts: missing.fonts,
            sandbox_profile: missing.sandbox,
        })
    }

    /// The pinned release's executable, to start: when the Host has it
    /// installed and has every library it loads; fonts the Host lacks do
    /// not keep it from starting (`browser.md` § Browser distribution).
    pub async fn executable(&self) -> Result<PathBuf> {
        let release = release()?;
        let platform = platform(&release)?;
        let installed = self
            .source()?
            .installed(ARTIFACT)
            .await
            .map_err(|error| BrowserError::Installation(error.to_string()))?;
        let path = installed
            .into_iter()
            .find(|installed| installed.sha256 == platform.sha256)
            .map(|installed| PathBuf::from(installed.path))
            .ok_or_else(|| {
                BrowserError::Installation(text::not_installed(&release.title(), platform.size))
            })?;
        let missing = self.missing(&path, &platform.executable).await?;
        if missing.libraries.is_empty() {
            return Ok(path);
        }
        let lines = text::requirements(&missing.libraries, &missing.fonts);
        Err(BrowserError::Installation(lines.trim_end().to_owned()))
    }

    /// What this Host lacks for the Chrome at `executable`, which is `entry`
    /// in its archive, read on the blocking pool: the check reads the
    /// Host's library and font directories.
    async fn missing(&self, executable: &Path, entry: &str) -> Result<Missing> {
        let requirements = self.requirements.clone();
        let executable = executable.to_owned();
        let entry = entry.to_owned();
        tokio::task::spawn_blocking(move || requirements(&executable, &entry)).await?
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

/// The release's archive for this Host.
fn platform(release: &BrowserRelease) -> Result<&ReleasePlatform> {
    release.platform(host_target()).ok_or_else(|| {
        BrowserError::Installation(format!(
            "{} is unavailable on {}",
            release.title(),
            host_target()
        ))
    })
}

/// The line `install` prints for `progress` of the pinned release
/// (`browser.md` § Installation).
pub fn progress_line(progress: ArtifactProgress) -> Result<String> {
    Ok(crate::driver::text::install_progress(&release()?.title(), progress))
}

/// The pinned release's version, such as `153.0.8010.36`.
pub fn pinned_version() -> Result<String> {
    Ok(release()?.version)
}
