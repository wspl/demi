//! The room on the host's disk for a large write to the store.
//!
//! A write that adds a large file, an image's import or a checkpoint's copy on
//! a store that cannot share extents, first checks that the host's disk holds
//! it with [`RESERVE`] to spare. When it does not, the writing Cloud's own
//! previous save goes first, since it is only a fallback; another device's
//! saves never go for it. When the room is still short, the write is refused,
//! so the disk never fills under the backend's databases and logs.
//!
//! The free bytes are the host's: a store in an XFS file on a loop device
//! (`crates/server/src/store.rs`) counts what its file may still grow into, and
//! the host may not have that, so the room is read from the filesystem that
//! holds the loop device's backing file.

use std::{io, path::Path};

use demi_machine_manager_protocol::{DeviceId, store};

use super::store::{ImageStore, StoreError};
use crate::blocking::OffLoop;

pub use demi_machine_manager_protocol::ROOM_RESERVE as RESERVE;

/// A write the host's disk has no room for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("Not enough disk space: needs {} more", gigabytes(self.short()))]
pub struct NoRoom {
    /// What the write adds.
    pub needed: u64,
    /// What the host had free.
    pub free: u64,
}

impl NoRoom {
    /// What the host lacked: the write and the reserve, less what was free.
    pub fn short(&self) -> u64 {
        self.needed.saturating_add(RESERVE).saturating_sub(self.free)
    }
}

/// Bytes as gigabytes with one decimal, as the pages show sizes.
fn gigabytes(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

#[derive(Debug, thiserror::Error)]
pub enum RoomError {
    #[error(transparent)]
    NoRoom(#[from] NoRoom),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// The bytes the writes that checked their room hold until they end: the
/// manager's devices write at once, and each check counts the others' bytes
/// as taken, so two copies that each fit alone never fill the disk together.
static RESERVED: std::sync::Mutex<u64> = std::sync::Mutex::new(0);

/// A write's bytes counted as taken until it ends, which dropping this does.
#[must_use]
pub struct Reserved(u64);

impl Reserved {
    /// Nothing counted, for a write too small to check.
    pub fn none() -> Self {
        Self(0)
    }
}

impl Drop for Reserved {
    fn drop(&mut self) {
        let mut reserved = RESERVED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        *reserved = reserved.saturating_sub(self.0);
    }
}

/// Makes sure the host's disk under `store` holds `needed` bytes more with
/// the reserve to spare, beside what other writes hold, calling `free_up`
/// while it does not; `free_up` answers whether it freed anything. The bytes
/// count as taken until the answer is dropped. Each check is one `statvfs`.
pub fn ensure(
    off: &OffLoop,
    store: &Path,
    needed: u64,
    mut free_up: impl FnMut(&OffLoop) -> io::Result<bool>,
) -> Result<Reserved, RoomError> {
    loop {
        let free = host_free(off, store)?;
        {
            let mut reserved = RESERVED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            let available = free.saturating_sub(*reserved);
            if available >= needed.saturating_add(RESERVE) {
                *reserved = reserved.saturating_add(needed);
                return Ok(Reserved(needed));
            }
        }
        if !free_up(off)? {
            tracing::warn!(event = "store_room.refused", needed, free, reserve = RESERVE, "a large write was refused: the host's disk lacks the room for it");
            let held = *RESERVED.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            return Err(NoRoom { needed, free: free.saturating_sub(held) }.into());
        }
    }
}

/// The room a copy for `device`'s store needs, for
/// [`clone_sparse`](crate::storage::clone::clone_sparse): its previous save
/// goes first when the host lacks it.
pub fn for_device(
    store: ImageStore,
    device: DeviceId,
) -> impl FnMut(&OffLoop, u64) -> io::Result<Reserved> + Send + 'static {
    move |off, needed| {
        let free_up = |off: &OffLoop| {
            store.drop_fallback(off, &device).map_err(|error| match error {
                StoreError::Io(error) => error,
                error => io::Error::other(error),
            })
        };
        ensure(off, store.root(), needed, free_up).map_err(|error| match error {
            RoomError::Io(error) => error,
            RoomError::NoRoom(refused) => io::Error::other(refused),
        })
    }
}

/// No room check, for a probe's few megabytes.
pub fn unchecked(_: &OffLoop, _: u64) -> io::Result<Reserved> {
    Ok(Reserved::none())
}

/// The bytes the host's disk under `path` has free: the filesystem's own, or,
/// for a filesystem on a loop device, that of the filesystem holding the loop
/// device's backing file.
pub fn host_free(_: &OffLoop, path: &Path) -> io::Result<u64> {
    let under = store::loop_holding(path)?
        .and_then(|(_, file)| file.parent().map(Path::to_owned))
        .unwrap_or_else(|| path.to_owned());
    let stats = rustix::fs::statvfs(&under)?;
    Ok(stats.f_bavail.saturating_mul(stats.f_frsize))
}

/// Requires that the store `data` can be worked on: no move of it is
/// unfinished, and a store whose XFS file exists is mounted from it, so the
/// manager never works on the directory beneath, which holds none of its
/// Clouds, or only part of them. Turns direct I/O on for that loop device.
pub fn require_mounted(off: &OffLoop, data: &Path) -> io::Result<()> {
    // The names `demi-server` gives, from the store's own path, whatever
    // link or trailing slash the configuration names it by.
    let data = match fs_err::canonicalize(data) {
        Ok(data) => data,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let data = data.as_path();
    let partial = store::partial_file(data);
    if partial.exists() {
        return Err(io::Error::other(format!(
            "a move of the Cloud store into {} is unfinished, so {} holds only part of it: `demi-server upgrade` finishes it",
            partial.display(),
            data.display()
        )));
    }
    let file = store::store_file(data);
    if !file.exists() {
        return Ok(());
    }
    if !store::mounted_from(data, &file)? {
        return Err(io::Error::other(format!(
            "the Cloud store {} is not mounted at {}, which holds none of its Clouds: its line in /etc/fstab mounts it",
            file.display(),
            data.display()
        )));
    }
    let (device, _) = store::loop_holding(data)?.expect("the store is mounted from its file");
    crate::linux::loopdev::set_direct_io(off, &device)
}
