//! Copying a machine image: a reflink clone where the filesystem supports
//! one, and otherwise a copy of only the ranges that hold data, skipping
//! holes and all-zero blocks, so unused capacity never becomes allocated
//! host storage. It runs in the manager's process, so the checkpoint's
//! frozen window starts no program.

use std::{io, os::unix::fs::FileExt, path::Path};

use fs_err::os::unix::fs::OpenOptionsExt;
use rustix::{fs::SeekFrom, io::Errno};

use crate::blocking::OffLoop;

/// The granularity of the zero-block check: a filesystem block.
const BLOCK: usize = 4096;

/// How much of a data range is read at once.
const CHUNK: usize = 1 << 20;

/// How a file was cloned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cloned {
    /// The copy shares the source's extents, and takes room only as either
    /// is written.
    Shared,
    /// The filesystem shares no extents, so the copy holds every data range
    /// again.
    Copied,
}

/// Copies `source` to the new file `destination`, mode 0600, keeping its
/// length and every byte it holds. Before a copy that shares no extents, it
/// asks `room` for the bytes the copy takes (`crate::storage::room`).
pub fn clone_sparse(
    off: &OffLoop,
    source: &Path,
    destination: &Path,
    room: &mut dyn FnMut(&OffLoop, u64) -> io::Result<super::room::Reserved>,
) -> io::Result<Cloned> {
    let from = fs_err::File::open(source)?;
    let to = fs_err::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(destination)?;
    match rustix::fs::ioctl_ficlone(to.file(), from.file()) {
        Ok(()) => return Ok(Cloned::Shared),
        // The filesystem cannot share extents between these files.
        Err(Errno::OPNOTSUPP | Errno::XDEV | Errno::INVAL | Errno::NOTTY | Errno::NOSYS) => {}
        Err(error) => return Err(crate::linux::failed("cloning to", destination, error)),
    }
    let metadata = from.metadata()?;
    let _reserved = room(off, std::os::unix::fs::MetadataExt::blocks(&metadata).saturating_mul(512))?;
    let length = metadata.len();
    copy_data(&from, &to, length)?;
    to.set_len(length)?;
    Ok(Cloned::Copied)
}

/// Copies each data range of `from`, leaving holes and all-zero blocks
/// unwritten.
fn copy_data(from: &fs_err::File, to: &fs_err::File, length: u64) -> io::Result<()> {
    let mut buffer = vec![0; CHUNK];
    let mut offset = 0;
    while offset < length {
        let data = match rustix::fs::seek(from.file(), SeekFrom::Data(offset)) {
            Ok(data) => data,
            // No data after `offset`: the rest is a hole.
            Err(Errno::NXIO) => break,
            Err(error) => return Err(error.into()),
        };
        let hole = rustix::fs::seek(from.file(), SeekFrom::Hole(data))?;
        let mut position = data;
        while position < hole {
            let count = usize::try_from(hole - position).map_or(CHUNK, |left| left.min(CHUNK));
            let chunk = &mut buffer[..count];
            from.file().read_exact_at(chunk, position)?;
            write_nonzero(to, chunk, position)?;
            position += count as u64;
        }
        offset = hole;
    }
    Ok(())
}

/// Writes the runs of `chunk`'s blocks that are not all zero.
fn write_nonzero(to: &fs_err::File, chunk: &[u8], position: u64) -> io::Result<()> {
    let mut run: Option<usize> = None;
    for (index, block) in chunk.chunks(BLOCK).enumerate() {
        let start = index * BLOCK;
        let zero = block.iter().all(|byte| *byte == 0);
        match (zero, run) {
            (false, None) => run = Some(start),
            (true, Some(first)) => {
                to.file()
                    .write_all_at(&chunk[first..start], position + first as u64)?;
                run = None;
            }
            _ => {}
        }
    }
    if let Some(first) = run {
        to.file()
            .write_all_at(&chunk[first..], position + first as u64)?;
    }
    Ok(())
}
