//! Discarding a mounted filesystem's free blocks (`managed-hosts.md` §
//! Images): `FITRIM` asks ext4 to discard every free block, which the loop
//! device beneath turns into holes punched in its image file, so a file the
//! Cloud deleted stops taking host storage.

use std::{io, os::fd::AsFd, path::Path};

use linux_raw_sys::{general::fstrim_range, ioctl::FITRIM};
use rustix::ioctl::Updater;

use super::failed;
use crate::blocking::OffLoop;

/// Discards the free blocks of the filesystem mounted at `mount`, the whole
/// of it; answers how many bytes the filesystem discarded.
pub fn trim(_: &OffLoop, mount: &Path) -> io::Result<u64> {
    let directory = fs_err::File::open(mount)?;
    let mut range = fstrim_range {
        start: 0,
        len: u64::MAX,
        minlen: 0,
    };
    // SAFETY: FITRIM reads an fstrim_range from the pointer and writes the
    // discarded length back into it; the descriptor is a filesystem's.
    unsafe {
        rustix::ioctl::ioctl(
            directory.file().as_fd(),
            Updater::<{ FITRIM }, fstrim_range>::new(&mut range),
        )
    }
    .map_err(|error| failed("trimming", mount, error))?;
    Ok(range.len)
}
