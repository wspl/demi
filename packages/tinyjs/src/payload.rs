//! Where the embedded bundle comes from.
//!
//! A packed binary carries the bundle as QuickJS bytecode in a section of
//! the executable, put there by `tinyjsc` through `libsui` (the injector
//! behind `deno compile`): a new segment on Mach-O, re-signed ad hoc; a
//! `PT_NOTE` on ELF. At start tinyjs asks `libsui::find_section` for it in
//! its own mapped image and loads it without parsing; nothing is read from
//! disk. The bare binary instead runs the entry file named on the command
//! line, with the entry's directory as the `/embedded/` namespace.
//!
//! Bytecode is tied to the interpreter build, so the section starts with a
//! header naming the release and abi it was compiled by, and every bare
//! binary carries a release marker `tinyjsc` reads before injecting.

use std::io;

use crate::runtime::ABI;

pub const ENTRY: &str = "/embedded/main.mjs";
pub const SECTION: &str = "TINYJS_BUNDLE";

/// The crate version: bytecode packed by one release runs only on that release.
pub const RELEASE: &str = env!("CARGO_PKG_VERSION");

/// What precedes the release in the marker. Spelled backwards so that the
/// only forward copy in any tinyjs binary is the marker itself, which is
/// what `tinyjsc` searches a bare binary's bytes for.
const MARKER_PREFIX_REVERSED: &[u8; 15] = b":ESAELER-SJYNIT";
const MARKER_LEN: usize = 64;

fn marker_prefix() -> [u8; 15] {
    reversed(MARKER_PREFIX_REVERSED)
}

const fn reversed<const N: usize>(bytes: &[u8; N]) -> [u8; N] {
    let mut out = [0u8; N];
    let mut i = 0;
    while i < N {
        out[i] = bytes[N - 1 - i];
        i += 1;
    }
    out
}

/// `TINYJS-RELEASE:<release>:<abi>\0`, kept in every tinyjs binary so the
/// packer can tell which release a bare binary is without running it.
#[used]
#[no_mangle]
pub static TINYJS_RELEASE_MARKER: [u8; MARKER_LEN] = marker();

const fn marker() -> [u8; MARKER_LEN] {
    let mut out = [0u8; MARKER_LEN];
    let prefix = reversed(MARKER_PREFIX_REVERSED);
    let mut n = 0;
    let mut i = 0;
    while i < prefix.len() {
        out[n] = prefix[i];
        n += 1;
        i += 1;
    }
    let release = RELEASE.as_bytes();
    i = 0;
    while i < release.len() {
        out[n] = release[i];
        n += 1;
        i += 1;
    }
    out[n] = b':';
    n += 1;
    let mut abi = ABI;
    let mut digits = [0u8; 10];
    let mut d = 0;
    loop {
        digits[d] = b'0' + (abi % 10) as u8;
        d += 1;
        abi /= 10;
        if abi == 0 {
            break;
        }
    }
    while d > 0 {
        d -= 1;
        out[n] = digits[d];
        n += 1;
    }
    out
}

/// Parses `<release>:<abi>` out of the bytes after the marker prefix.
fn parse_marker(rest: &[u8]) -> Option<(String, u32)> {
    let end = rest.iter().position(|&b| b == 0)?;
    let text = std::str::from_utf8(&rest[..end]).ok()?;
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-' || b == b'+' || b == b':') {
        return None;
    }
    let (release, abi) = text.rsplit_once(':')?;
    Some((release.to_string(), abi.parse().ok()?))
}

/// This binary's release and abi, read from its own marker (which also
/// keeps the marker referenced, so no linker drops it).
pub fn own_release() -> (String, u32) {
    parse_marker(&TINYJS_RELEASE_MARKER[MARKER_PREFIX_REVERSED.len()..]).expect("own release marker")
}

/// The release and abi of a bare tinyjs binary, from its marker.
pub fn release_of(exe: &[u8]) -> io::Result<(String, u32)> {
    let prefix = marker_prefix();
    let mut from = 0;
    while let Some(at) = exe[from..].windows(prefix.len()).position(|w| w == prefix) {
        let start = from + at + prefix.len();
        if let Some(found) = parse_marker(&exe[start..(start + MARKER_LEN).min(exe.len())]) {
            return Ok(found);
        }
        from = start;
    }
    Err(io::Error::other("not a tinyjs binary: no release marker"))
}

/// The section magic, backwards for the same reason as the marker prefix:
/// a forward copy in a binary means a section was injected.
const MAGIC_REVERSED: &[u8; 8] = b"1LDNBSJT";

/// The section's magic.
pub fn magic() -> [u8; 8] {
    reversed(MAGIC_REVERSED)
}

/// The section: magic, abi (u32 LE), release length (u8), release, bytecode.
pub fn encode_section(bytecode: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + 5 + RELEASE.len() + bytecode.len());
    out.extend_from_slice(&magic());
    out.extend_from_slice(&ABI.to_le_bytes());
    out.push(RELEASE.len() as u8);
    out.extend_from_slice(RELEASE.as_bytes());
    out.extend_from_slice(bytecode);
    out
}

/// The bytecode of a section this release can run; anything else is refused
/// here rather than fed to the interpreter.
pub fn decode_section(section: &'static [u8]) -> io::Result<&'static [u8]> {
    let bad = |what: &str| io::Error::other(format!("packed bundle {what}"));
    if section.len() < 13 || section[..8] != magic() {
        return Err(bad("has no tinyjsc header"));
    }
    let abi = u32::from_le_bytes(section[8..12].try_into().unwrap());
    let len = section[12] as usize;
    let release = std::str::from_utf8(section.get(13..13 + len).ok_or_else(|| bad("header is truncated"))?).map_err(|_| bad("header is not text"))?;
    let (own, own_abi) = own_release();
    if release != own || abi != own_abi {
        return Err(bad(&format!("was packed by tinyjsc {release} (abi {abi}); this is tinyjs {own} (abi {own_abi})")));
    }
    Ok(&section[13 + len..])
}

pub enum Payload {
    /// The bundle packed into the executable, as bytecode.
    Packed { bytecode: &'static [u8] },
    /// A directory on disk standing in for `/embedded/`.
    Directory { dir: String, entry: String },
}

/// The entry module as the loader receives it.
pub enum Entry {
    Bytecode(&'static [u8]),
    Source(Vec<u8>),
}

impl Payload {
    /// The name under which the entry module is evaluated.
    pub fn entry_name(&self) -> String {
        match self {
            Payload::Packed { .. } => ENTRY.to_string(),
            Payload::Directory { entry, .. } => entry.clone(),
        }
    }

    pub fn entry(&self) -> io::Result<Entry> {
        match self {
            Payload::Packed { bytecode } => Ok(Entry::Bytecode(bytecode)),
            Payload::Directory { dir, entry } => {
                let name = entry.strip_prefix("/embedded/").unwrap_or(entry);
                Ok(Entry::Source(std::fs::read(format!("{dir}/{name}"))?))
            }
        }
    }

    /// Resolves an `/embedded/…` import to its source. Only the directory
    /// payload has modules besides the entry.
    pub fn source(&self, name: &str) -> Option<io::Result<Vec<u8>>> {
        let rest = name.strip_prefix("/embedded/")?;
        match self {
            Payload::Packed { .. } => None,
            Payload::Directory { dir, .. } => Some(std::fs::read(format!("{dir}/{rest}"))),
        }
    }
}

/// The packed bundle's bytecode, if the executable carries one.
pub fn packed() -> io::Result<Option<&'static [u8]>> {
    libsui::find_section(SECTION)?.map(decode_section).transpose()
}

/// Maps a command-line entry path to the directory payload.
pub fn directory(entry_path: &str) -> io::Result<Payload> {
    let path = std::fs::canonicalize(entry_path)?;
    let dir = path.parent().unwrap_or(std::path::Path::new("/")).to_string_lossy().into_owned();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Payload::Directory { dir, entry: format!("/embedded/{name}") })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn own_marker_names_this_release() {
        assert_eq!(own_release(), (RELEASE.to_string(), ABI));
        assert_eq!(release_of(&TINYJS_RELEASE_MARKER).unwrap(), (RELEASE.to_string(), ABI));
    }

    #[test]
    fn section_round_trips_and_refuses_other_releases() {
        let section: &'static [u8] = Box::leak(encode_section(b"bytecode").into_boxed_slice());
        assert_eq!(decode_section(section).unwrap(), b"bytecode");
        let mut other = encode_section(b"x");
        other[8] ^= 1;
        let other: &'static [u8] = Box::leak(other.into_boxed_slice());
        assert!(decode_section(other).unwrap_err().to_string().contains("abi"));
        assert!(decode_section(b"garbage-without-a-header").unwrap_err().to_string().contains("header"));
    }
}
