//! The name of a runner installation's directory, which the installers and the
//! runner both derive from the backend's URL, so that no two of them can
//! disagree about it: `~/.demi/runner/<name>`.
//!
//! The name is the backend's host, then its port when it is not the scheme's
//! default, then the URL's path segments, joined by `-`, and last the first
//! four hex digits of the SHA-256 of the whole URL: `demi.example.com-3f9a`,
//! `127.0.0.1-3310-b27e`. A person reads from it which backend an installation
//! serves, and the hash keeps apart the backends the readable part cannot, such
//! as `http://example.com` and `https://example.com`, or `/team-a` and
//! `/team/a`. The URL is hashed as it is written, so `/Team` and `/team`, which
//! a case-insensitive file system would otherwise read as one directory, are
//! two names.
//!
//! The name is a safe file name on every system. In the readable part only
//! ASCII letters and digits, `.`, `-` and `_` stay; every other character, such
//! as the `:` of an IPv6 host or a percent-encoded byte's `%`, becomes `_`. The
//! hash also ends the name, so no name is a device name Windows reserves, and
//! none ends in a dot.
//!
//! Two backends whose names agree up to the hash would share a directory, one
//! chance in 65,536 when their readable parts agree.

use sha2::{Digest, Sha256};
use url::{Host, Url};

/// The directories under the user's home that hold every installation.
pub const ROOT: [&str; 2] = [".demi", "runner"];

/// The directory name of the installation of `backend`.
pub fn installation_name(backend: &Url) -> String {
    let mut parts: Vec<String> = Vec::new();
    match backend.host() {
        Some(Host::Domain(domain)) => parts.push(domain.to_owned()),
        Some(Host::Ipv4(address)) => parts.push(address.to_string()),
        Some(Host::Ipv6(address)) => parts.push(address.to_string()),
        None => {}
    }
    if let Some(port) = backend.port() {
        parts.push(port.to_string());
    }
    if let Some(segments) = backend.path_segments() {
        parts.extend(segments.filter(|segment| !segment.is_empty()).map(str::to_owned));
    }
    let readable: String = parts
        .join("-")
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    let digest = Sha256::digest(backend.as_str().as_bytes());
    format!("{readable}-{:02x}{:02x}", digest[0], digest[1])
}
