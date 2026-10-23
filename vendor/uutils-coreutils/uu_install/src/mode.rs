// This file is part of the uutils coreutils package.
//
// For the full copyright and license information, please view the LICENSE
// file that was distributed with this source code.
use std::path::Path;
#[cfg(unix)]
use uucore::context::fs;
#[cfg(unix)]
use uucore::translate;

/// chmod a file or directory on Windows, which has no Unix mode: it changes
/// nothing, as `chmod` does there.
#[cfg(windows)]
pub fn chmod(_path: &Path, _mode: u32) -> Result<(), ()> {
    Ok(())
}

/// What a Windows file's metadata says of the Unix mode and owner it has none
/// of, as `uucore::fs::portable_mode` and a zero owner.
#[cfg(windows)]
pub trait MetadataExt {
    fn mode(&self) -> u32;
    fn uid(&self) -> u32;
    fn gid(&self) -> u32;
}

#[cfg(windows)]
impl MetadataExt for std::fs::Metadata {
    fn mode(&self) -> u32 {
        uucore::fs::portable_mode(self)
    }
    fn uid(&self) -> u32 {
        0
    }
    fn gid(&self) -> u32 {
        0
    }
}

/// chmod a file or directory on UNIX.
///
/// Adapted from mkdir.rs.  Handles own error printing.
#[cfg(unix)]
pub fn chmod(path: &Path, mode: u32) -> Result<(), ()> {
    use std::os::unix::fs::PermissionsExt;
    use uucore::{display::Quotable, show_error};
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|err| {
        show_error!(
            "{}",
            translate!("install-error-chmod-failed-detailed", "path" => path.maybe_quote(), "error" => err)
        );
    })
}
