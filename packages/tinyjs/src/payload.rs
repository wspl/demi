//! Where the embedded bundle comes from.
//!
//! A packed binary carries the bundle in a slot reserved inside the
//! executable at build time: a static block that starts with a magic
//! header, the slot's capacity and the bundle length, followed by the
//! capacity's worth of bytes. Packing finds the magic in the file and
//! writes the length and the bundle in place, so no segment moves, no
//! format is parsed and a code signature can be re-applied afterwards
//! without strict validation complaining. At start tinyjs reads the length
//! from its own mapped image; nothing is read from disk. The bare binary
//! (length zero) instead runs the entry file named on the command line,
//! with the entry's directory as the `/embedded/` namespace.

use std::io;

pub const ENTRY: &str = "/embedded/main.mjs";

/// Fixed at build time; a bundle larger than this fails to pack.
pub const CAPACITY: usize = 8 * 1024 * 1024;

/// What a packer searches for. The magic is followed by the capacity and
/// the bundle length, both little-endian `u64`, then the bundle bytes.
pub const MAGIC: [u8; 16] = *b"TINYJS_SLOT_v1__";

#[repr(C)]
struct Slot {
    magic: [u8; 16],
    capacity: u64,
    length: u64,
    data: [u8; CAPACITY],
}

// A non-zero header keeps the whole slot in a file-backed section rather
// than in zero-fill memory, which is what lets a packer write into it. It is
// placed in the writable data segment, after the code, so that the code's
// first-execution page faults never read through 8 MiB of zeros: measured
// in the guest, a slot in `.rodata` ahead of the code cost 70 ms.
#[used]
#[cfg_attr(target_os = "macos", link_section = "__DATA,__tinyjs_slot")]
#[cfg_attr(target_os = "linux", link_section = ".data.tinyjs_slot")]
static SLOT: Slot = Slot { magic: MAGIC, capacity: CAPACITY as u64, length: 0, data: [0; CAPACITY] };

pub enum Payload {
    /// The bundle packed into the executable.
    Packed { source: &'static [u8] },
    /// A directory on disk standing in for `/embedded/`.
    Directory { dir: String, entry: String },
}

impl Payload {
    /// The name under which the entry module is evaluated.
    pub fn entry_name(&self) -> String {
        match self {
            Payload::Packed { .. } => ENTRY.to_string(),
            Payload::Directory { entry, .. } => entry.clone(),
        }
    }

    /// Resolves an `/embedded/…` name to a source, if this payload has it.
    pub fn source(&self, name: &str) -> Option<io::Result<Vec<u8>>> {
        let rest = name.strip_prefix("/embedded/")?;
        match self {
            Payload::Packed { source } => (name == ENTRY).then(|| Ok(source.to_vec())),
            Payload::Directory { dir, .. } => Some(std::fs::read(format!("{dir}/{rest}"))),
        }
    }
}

/// The packed bundle, if the executable carries one.
pub fn packed() -> Option<&'static [u8]> {
    // Volatile: the value in the file is what counts, not the initialiser.
    // SAFETY: reading a field of a static.
    let length = unsafe { std::ptr::read_volatile(&SLOT.length) } as usize;
    if length == 0 {
        return None;
    }
    Some(&SLOT.data[..length.min(CAPACITY)])
}

/// Maps a command-line entry path to the directory payload.
pub fn directory(entry_path: &str) -> io::Result<Payload> {
    let path = std::fs::canonicalize(entry_path)?;
    let dir = path.parent().unwrap_or(std::path::Path::new("/")).to_string_lossy().into_owned();
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(Payload::Directory { dir, entry: format!("/embedded/{name}") })
}
