//! What Chrome needs on a Linux Host that Demi does not install
//! (`browser.md` § Installation): the libraries and fonts of the package's
//! record that the Host lacks, and, on a Host that restricts user namespaces
//! with AppArmor, the profile that lets Chrome's sandbox start. `install`
//! names them, and a command that would start Chrome on a Host without the
//! libraries names those; neither installs any.

use std::path::Path;

use demi_command_package_browser_protocol::browser::SandboxProfile;
use demi_command_package_browser_protocol::release::{LinuxFont, LinuxLibrary};

#[cfg(target_os = "linux")]
use demi_command_package_browser_protocol::release::LinuxRequirements;

#[cfg(target_os = "linux")]
use crate::driver::operation::BrowserError;
use crate::driver::operation::Result;

/// The record's libraries and fonts this Host lacks, and the AppArmor
/// profile its sandbox lacks.
#[derive(Debug, Default, Clone)]
pub struct Missing {
    pub libraries: Vec<LinuxLibrary>,
    pub fonts: Vec<LinuxFont>,
    pub sandbox: Option<SandboxProfile>,
}

/// What this Host lacks for the Chrome at `executable`, which is `entry`
/// in its archive.
#[cfg(target_os = "linux")]
pub fn missing(executable: &Path, entry: &str) -> Result<Missing> {
    let requirements = LinuxRequirements::pinned()
        .map_err(|error| BrowserError::Configuration(error.to_string()))?;
    Ok(linux::missing(requirements, executable, entry))
}

/// Nothing: off Linux, Chrome brings what it needs.
#[cfg(not(target_os = "linux"))]
pub fn missing(_executable: &Path, _entry: &str) -> Result<Missing> {
    Ok(Missing::default())
}

#[cfg(target_os = "linux")]
mod linux {
    use std::collections::HashSet;
    use std::ffi::OsString;
    use std::path::{Path, PathBuf};

    use demi_command_package_browser_protocol::browser::SandboxProfile;

    use demi_command_package_browser_protocol::release::LinuxRequirements;

    use super::Missing;

    /// The directories the dynamic linker searches by default, and those
    /// `/etc/ld.so.conf.d` adds, which name the multiarch ones on Ubuntu.
    const LIBRARY_DIRECTORIES: &[&str] = &["/lib", "/usr/lib", "/lib64", "/usr/lib64"];
    const LINKER_CONFIGURATION: &str = "/etc/ld.so.conf.d";
    /// The system's font directories; the user's lie under their home.
    const FONT_DIRECTORIES: &[&str] = &["/usr/share/fonts", "/usr/local/share/fonts"];
    const USER_FONT_DIRECTORIES: &[&str] = &[".local/share/fonts", ".fonts"];

    /// Whether the kernel keeps unprivileged processes from making user
    /// namespaces unless an AppArmor profile allows them, as Ubuntu 23.10
    /// and later do.
    const RESTRICTION: &str = "/proc/sys/kernel/apparmor_restrict_unprivileged_userns";
    /// The profile `install` suggests, by its name and its file.
    const PROFILE: &str = "demi-chrome";
    const PROFILE_FILE: &str = "/etc/apparmor.d/demi-chrome";

    pub(super) fn missing(requirements: LinuxRequirements, executable: &Path, entry: &str) -> Missing {
        let libraries = library_directories();
        let fonts = font_files();
        Missing {
            sandbox: sandbox(executable, entry),
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

    /// The profile Chrome's sandbox needs, unless the Host does not
    /// restrict user namespaces or its profiles hold the one for this
    /// Chrome. Only root may read which profiles the kernel loaded; AppArmor
    /// loads `/etc/apparmor.d` at boot, and the commands `install` prints
    /// write and load the profile together.
    /// The profile allows every version of the line this runner installs:
    /// each lies at `entry` in a directory of the cache named by its digest.
    fn sandbox(executable: &Path, entry: &str) -> Option<SandboxProfile> {
        // A Host without the setting does not restrict user namespaces.
        let restricted = std::fs::read_to_string(RESTRICTION).is_ok_and(|value| value.trim() == "1");
        if !restricted {
            return None;
        }
        let depth = Path::new(entry).components().count() + 1;
        let cache = executable.ancestors().nth(depth)?;
        let attachment = format!("{}/*/{entry}", cache.display());
        let profile = format!(
            "abi <abi/4.0>,\ninclude <tunables/global>\n\nprofile {PROFILE} {attachment} flags=(unconfined) {{\n  userns,\n\n  include if exists <local/{PROFILE}>\n}}\n"
        );
        // An unreadable file is reported missing, which the user can check.
        let written = std::fs::read_to_string(PROFILE_FILE).is_ok_and(|text| text.contains(&attachment));
        if written {
            return None;
        }
        Some(SandboxProfile {
            path: PROFILE_FILE.to_owned(),
            profile,
        })
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
