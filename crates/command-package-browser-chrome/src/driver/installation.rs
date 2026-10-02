//! The pinned Chrome for Testing release (`browser.md` § Browser
//! distribution): its version, and the executable the runner installed as
//! `demi.browser`'s `chrome` resource and named when it started the
//! program. Nothing here downloads Chrome or looks for it.

use std::path::{Path, PathBuf};

use demi_command_package_browser_protocol::release::BrowserRelease;

use crate::driver::operation::{BrowserError, Result};

/// The Chrome a service starts: the executable it was given, if any.
#[derive(Debug, Clone)]
pub struct Chrome {
    executable: Option<PathBuf>,
}

impl Chrome {
    /// The Chrome whose executable is `executable`, the `chrome` resource's
    /// entry; without one, every browser fails to start.
    pub fn new(executable: Option<PathBuf>) -> Self {
        Self { executable }
    }

    /// The executable a browser starts.
    pub fn executable(&self) -> Result<&Path> {
        self.executable.as_deref().ok_or_else(|| {
            BrowserError::Installation(
                "demi-browser was started without its chrome resource".to_owned(),
            )
        })
    }

    /// Where Chrome's executables live, its helpers' included, for finding
    /// its processes.
    pub fn roots(&self) -> Vec<PathBuf> {
        self.executable
            .as_deref()
            .map(installation_of)
            .into_iter()
            .collect()
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

/// The pinned release's version, such as `153.0.8010.36`.
pub fn pinned_version() -> Result<String> {
    BrowserRelease::pinned()
        .map(|release| release.version)
        .map_err(|error| BrowserError::Configuration(error.to_string()))
}
