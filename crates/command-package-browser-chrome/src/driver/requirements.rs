//! What Chrome needs on a Linux Host that Demi does not install
//! (`browser.md` § Installation): the libraries and fonts of the package's
//! record that the Host lacks. `install` names them; it installs none.

use demi_command_package_browser_protocol::release::{LinuxFont, LinuxLibrary};

#[cfg(target_os = "linux")]
use demi_command_package_browser_protocol::release::LinuxRequirements;

#[cfg(target_os = "linux")]
use crate::driver::operation::BrowserError;
use crate::driver::operation::Result;

/// The record's libraries and fonts this Host lacks.
#[derive(Debug, Default)]
pub struct Missing {
    pub libraries: Vec<LinuxLibrary>,
    pub fonts: Vec<LinuxFont>,
}

/// What this Host lacks of the record.
#[cfg(target_os = "linux")]
pub fn missing() -> Result<Missing> {
    let requirements = LinuxRequirements::pinned()
        .map_err(|error| BrowserError::Configuration(error.to_string()))?;
    Ok(linux::missing(requirements))
}

/// Nothing: off Linux, Chrome brings what it needs.
#[cfg(not(target_os = "linux"))]
pub fn missing() -> Result<Missing> {
    Ok(Missing::default())
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::HashSet;
    use std::ffi::OsString;
    use std::path::PathBuf;

    use demi_command_package_browser_protocol::release::LinuxRequirements;

    use super::Missing;

    /// The directories the dynamic linker searches by default, and those
    /// `/etc/ld.so.conf.d` adds, which name the multiarch ones on Ubuntu.
    const LIBRARY_DIRECTORIES: &[&str] = &["/lib", "/usr/lib", "/lib64", "/usr/lib64"];
    const LINKER_CONFIGURATION: &str = "/etc/ld.so.conf.d";
    /// The system's font directories; the user's lie under their home.
    const FONT_DIRECTORIES: &[&str] = &["/usr/share/fonts", "/usr/local/share/fonts"];
    const USER_FONT_DIRECTORIES: &[&str] = &[".local/share/fonts", ".fonts"];

    pub(super) fn missing(requirements: LinuxRequirements) -> Missing {
        let libraries = library_directories();
        let fonts = font_files();
        Missing {
            libraries: requirements
                .libraries
                .into_iter()
                .filter(|library| {
                    !libraries
                        .iter()
                        .any(|directory| directory.join(&library.name).exists())
                })
                .collect(),
            fonts: requirements
                .fonts
                .into_iter()
                .filter(|font| !fonts.contains(&OsString::from(&font.file)))
                .collect(),
        }
    }

    fn library_directories() -> Vec<PathBuf> {
        let mut directories: Vec<PathBuf> = LIBRARY_DIRECTORIES.iter().map(PathBuf::from).collect();
        // A Host without the directory, or with a file it cannot read, adds
        // nothing: a library only it would name is then reported missing,
        // which the user can check.
        let Ok(entries) = std::fs::read_dir(LINKER_CONFIGURATION) else {
            return directories;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().is_none_or(|extension| extension != "conf") {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            directories.extend(
                text.lines()
                    .map(str::trim)
                    .filter(|line| line.starts_with('/'))
                    .map(PathBuf::from),
            );
        }
        directories
    }

    /// The names of the font files in the system's and the user's font
    /// directories.
    fn font_files() -> HashSet<OsString> {
        let home = std::env::home_dir();
        let user = USER_FONT_DIRECTORIES
            .iter()
            .filter_map(|directory| home.as_ref().map(|home| home.join(directory)));
        let directories = FONT_DIRECTORIES.iter().map(PathBuf::from).chain(user);
        // An unreadable entry hides only the fonts beneath it, which are
        // then reported missing.
        directories
            .flat_map(|directory| walkdir::WalkDir::new(directory).into_iter().flatten())
            .filter(|entry| entry.file_type().is_file())
            .map(|entry| entry.file_name().to_owned())
            .collect()
    }
}
