//! The distribution platform key of the machine this package runs on.

use std::path::Path;

/// Which dynamic loaders the machine has for its architecture.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Loaders {
    pub musl: bool,
    pub glibc: bool,
}

/// The platform key for an operating system and architecture, as named by
/// `std::env::consts`. A Linux machine takes the `-musl` build only when it has
/// a musl loader and no glibc loader: this package is a static musl build that
/// also runs on glibc machines, and the two CLI builds are not interchangeable.
pub fn platform_key(os: &str, arch: &str, loaders: Loaders) -> Option<&'static str> {
    let musl = loaders.musl && !loaders.glibc;
    Some(match (os, arch, musl) {
        ("macos", "aarch64", _) => "darwin-arm64",
        ("macos", "x86_64", _) => "darwin-x64",
        ("windows", "x86_64", _) => "win32-x64",
        ("windows", "aarch64", _) => "win32-arm64",
        ("linux", "x86_64", false) => "linux-x64",
        ("linux", "x86_64", true) => "linux-x64-musl",
        ("linux", "aarch64", false) => "linux-arm64",
        ("linux", "aarch64", true) => "linux-arm64-musl",
        _ => return None,
    })
}

/// This machine's platform key, or `None` when the CLI has no build for it.
pub fn current() -> Option<&'static str> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let loaders = if os == "linux" {
        let (musl, glibc) = match arch {
            "x86_64" => ("/lib/ld-musl-x86_64.so.1", "/lib64/ld-linux-x86-64.so.2"),
            "aarch64" => ("/lib/ld-musl-aarch64.so.1", "/lib/ld-linux-aarch64.so.1"),
            _ => return None,
        };
        Loaders {
            musl: Path::new(musl).exists(),
            glibc: Path::new(glibc).exists(),
        }
    } else {
        Loaders::default()
    };
    platform_key(os, arch, loaders)
}
