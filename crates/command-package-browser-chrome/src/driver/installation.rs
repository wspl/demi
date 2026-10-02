//! The pinned Chrome for Testing release (`browser.md` § Browser
//! distribution): its version, and its executable, which the runner
//! installs when the program asks for it over its artifacts stream
//! (`native-runtime.md` § The artifacts stream). Nothing here downloads
//! Chrome or looks for it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use demi_command_package_browser_protocol::release::BrowserRelease;
use demi_command_protocol::{ArtifactForm, ArtifactInstall, host_target};
use demi_command_sdk::Artifacts;
use tokio::sync::watch;

use crate::driver::operation::{BrowserError, Result};

/// The artifact line of the pinned release.
pub const NAME: &str = "Chrome for Testing";

/// The Chrome a service starts, through the runner that installs it.
/// Cloning shares the one source.
#[derive(Clone)]
pub struct Chrome {
    artifacts: Arc<watch::Sender<Option<Artifacts>>>,
}

impl Default for Chrome {
    fn default() -> Self {
        Self {
            artifacts: Arc::new(watch::Sender::new(None)),
        }
    }
}

impl Chrome {
    /// Takes the service's artifacts source, which the runner answers.
    pub fn attach(&self, artifacts: Artifacts) {
        self.artifacts.send_replace(Some(artifacts));
    }

    /// The pinned release's executable for this Host, which the runner
    /// installs for `invocation` when it has none.
    pub async fn executable(&self, invocation: &str) -> Result<PathBuf> {
        let release = release()?;
        let platform = release.platform(host_target()).ok_or_else(|| {
            BrowserError::Installation(format!(
                "{} is unavailable on {}",
                release.title(),
                host_target()
            ))
        })?;
        let install = ArtifactInstall {
            invocation: invocation.to_owned(),
            name: NAME.to_owned(),
            version: release.version.clone(),
            sha256: platform.sha256.clone(),
            size: platform.size,
            form: ArtifactForm::Archive {
                entry: platform.executable.clone(),
            },
        };
        self.source()?
            .install(install)
            .await
            .map_err(|error| BrowserError::Installation(error.to_string()))
    }

    /// Where Chrome's executables live, its helpers' included, for finding
    /// its processes: each installation of the line the runner has, once
    /// the service has its artifacts source.
    pub async fn roots(&self) -> Vec<PathBuf> {
        let mut attached = self.artifacts.subscribe();
        // The sender lives as long as this Chrome, so the wait ends only
        // with a source.
        let Ok(artifacts) = attached.wait_for(Option::is_some).await else {
            return Vec::new();
        };
        let artifacts = artifacts.clone().expect("the wait ends with a source");
        match artifacts.installed(NAME).await {
            Ok(installed) => installed
                .iter()
                .map(|installed| installation_of(Path::new(&installed.path)))
                .collect(),
            Err(error) => {
                tracing::warn!("the installed Chrome could not be listed: {error}");
                Vec::new()
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

/// The pinned release's version, such as `153.0.8010.36`.
pub fn pinned_version() -> Result<String> {
    Ok(release()?.version)
}
