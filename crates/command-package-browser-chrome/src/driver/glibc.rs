//! The Host's glibc version, which the Chrome runtime needs at 2.28 or newer
//! (`browser.md` § Browser distribution). This program is a static musl
//! executable with no glibc of its own to ask, so it reads the version from
//! the glibc dynamic loader that Chrome's executable names as its
//! interpreter.

use std::fmt;
use std::io;
use std::path::Path;

/// A glibc version, such as `2.28`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct GlibcVersion {
    pub major: u32,
    pub minor: u32,
}

/// The oldest glibc the Chrome runtime runs on.
pub const OLDEST: GlibcVersion = GlibcVersion {
    major: 2,
    minor: 28,
};

impl fmt::Display for GlibcVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

impl GlibcVersion {
    /// The version at the start of `text`, such as `2.39` of `2.39.` or of
    /// `2.39.9000`.
    fn parse(text: &str) -> Option<Self> {
        let mut parts = text.split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        Some(Self { major, minor })
    }
}

/// The glibc dynamic loader of this architecture, at the path Chrome for
/// Testing's executable names.
fn loader() -> Option<&'static Path> {
    match std::env::consts::ARCH {
        "x86_64" => Some(Path::new("/lib64/ld-linux-x86-64.so.2")),
        "aarch64" => Some(Path::new("/lib/ld-linux-aarch64.so.1")),
        _ => None,
    }
}

/// The Host's glibc version; none on a Host without glibc's loader, such as
/// one with musl alone.
pub fn host() -> io::Result<Option<GlibcVersion>> {
    let Some(loader) = loader() else {
        return Ok(None);
    };
    let resolved = match std::fs::canonicalize(loader) {
        Ok(resolved) => resolved,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let named = resolved
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(from_file_name);
    if let Some(version) = named {
        return Ok(Some(version));
    }
    let bytes = std::fs::read(&resolved)?;
    from_banner(&bytes).map(Some).ok_or_else(|| {
        io::Error::other(format!(
            "{} names no glibc version",
            resolved.display()
        ))
    })
}

/// The version of a loader that glibc before 2.34 names by it, such as
/// `ld-2.26.so`.
fn from_file_name(name: &str) -> Option<GlibcVersion> {
    GlibcVersion::parse(name.strip_prefix("ld-")?.strip_suffix(".so")?)
}

/// The version in the loader's banner, which glibc 2.33 and later print for
/// `--version`, such as `… stable release version 2.39.`.
fn from_banner(bytes: &[u8]) -> Option<GlibcVersion> {
    const MARK: &[u8] = b"release version ";
    let start = bytes
        .windows(MARK.len())
        .position(|window| window == MARK)?
        + MARK.len();
    let version: Vec<u8> = bytes[start..]
        .iter()
        .take_while(|byte| byte.is_ascii_digit() || **byte == b'.')
        .copied()
        .collect();
    GlibcVersion::parse(std::str::from_utf8(&version).ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A loader says its version in its name before glibc 2.34, as on
    /// Amazon Linux 2 and Debian 10, and in its banner from 2.33, as on
    /// Debian 12 and Fedora.
    #[test]
    fn a_loader_says_its_version_by_its_name_or_its_banner() {
        let version = |major, minor| Some(GlibcVersion { major, minor });
        assert_eq!(from_file_name("ld-2.26.so"), version(2, 26));
        assert_eq!(from_file_name("ld-linux-aarch64.so.1"), None);
        let banner = b"\0ld.so (Debian GLIBC 2.36-9) stable release version 2.36.\nCopyright";
        assert_eq!(from_banner(banner), version(2, 36));
        assert_eq!(from_banner(b"release version 2.43.9000\0"), version(2, 43));
        assert_eq!(from_banner(b"no banner"), None);
        assert!(version(2, 26) < Some(OLDEST) && version(2, 36) > Some(OLDEST));
        assert_eq!(OLDEST.to_string(), "2.28");
    }
}
