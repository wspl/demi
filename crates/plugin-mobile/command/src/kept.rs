//! What the services keep across their own ends
//! (`crates/plugin-mobile/command/src/devices.rs`), in
//! `<installation>/mobile/kept.json`:
//!
//! - the simulators Demi booted, so a simulator is shut down with its
//!   device only when Demi booted it. Each names the boot by its mark
//!   (`driver::Simulator::boot`), so a simulator the user shut down and
//!   booted again is the user's.
//! - the devices open, which conversation has each and under which handle,
//!   and which service owns it, so a service that starts again takes each
//!   back under the same handle
//!   (`docs/decisions/0031-a-device-tab-lasts-as-long-as-the-device.md`).
//!
//! A `demi-mobile` that stops, updates or is killed shuts nothing down and
//! forgets nothing; a device leaves the record when it closes or is lost.
//!
//! # Shared by processes
//!
//! Two versions of the service can run at once, as when a runner's update
//! leaves the older one holding the conversations it has. The file is
//! therefore never written from what one process remembers: every change
//! holds the OS lock of `kept.lock`, reads the file fresh, changes the
//! entries it means to and writes the whole through a temporary file of its
//! own. A service changes only the devices it owns, and takes a
//! conversation's devices over, by [`Kept::claim`], only once the
//! conversation reaches it.

use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
};

use serde::{Deserialize, Serialize};

use demi_command_package_mobile_protocol::mobile::{DeviceId, MobilePlatform};

/// The record's file in the installation's `mobile/`.
const FILE: &str = "kept.json";
/// The file whose OS lock orders the changes of every process.
const LOCK: &str = "kept.lock";

/// A simulator Demi booted, and the boot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Entry {
    #[garde(skip)]
    pub platform: MobilePlatform,
    #[garde(length(min = 1))]
    pub native_id: String,
    /// The boot's mark, as the driver lists it.
    #[garde(skip)]
    pub boot: u64,
}

/// A device a conversation has open.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Opened {
    #[garde(skip)]
    pub handle: DeviceId,
    #[garde(length(min = 1))]
    pub conversation: String,
    #[garde(skip)]
    pub platform: MobilePlatform,
    #[garde(length(min = 1))]
    pub native_id: String,
    /// The service that holds it now.
    #[garde(length(min = 1))]
    pub owner: String,
}

/// The file's content.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
struct Record {
    #[garde(dive)]
    simulators: Vec<Entry>,
    #[garde(dive)]
    devices: Vec<Opened>,
}

/// The record: the installation's file, or, for a program started without
/// an installation, which keeps nothing across its end, memory.
pub struct Kept {
    directory: Option<PathBuf>,
    memory: Mutex<Record>,
}

impl Kept {
    /// The record in `mobile` (the installation's `mobile/`). A file that
    /// does not read is reported and replaced at the next change: what it
    /// named is the user's to shut down, and the devices it named are not
    /// open any more.
    pub fn load(mobile: Option<&Path>) -> Self {
        Self {
            directory: mobile.map(Path::to_owned),
            memory: Mutex::default(),
        }
    }

    /// Whether Demi booted `native_id` of `platform` in its boot `boot`.
    pub fn holds(&self, platform: MobilePlatform, native_id: &str, boot: Option<u64>) -> bool {
        boot.is_some_and(|boot| {
            self.edit(|record| {
                record
                    .simulators
                    .iter()
                    .any(|entry| entry.platform == platform && entry.native_id == native_id && entry.boot == boot)
            })
        })
    }

    /// Demi booted `entry`'s simulator; an older boot of it leaves.
    pub fn booted(&self, entry: Entry) {
        self.edit(|record| {
            record
                .simulators
                .retain(|kept| !(kept.platform == entry.platform && kept.native_id == entry.native_id));
            record.simulators.push(entry);
        });
    }

    /// `native_id` of `platform` is no longer Demi's to shut down.
    pub fn unbooted(&self, platform: MobilePlatform, native_id: &str) {
        self.edit(|record| {
            record
                .simulators
                .retain(|kept| !(kept.platform == platform && kept.native_id == native_id));
        });
    }

    /// The service `owner` holds `device`: any older entry of the simulator
    /// is replaced, whoever held it.
    pub fn opened(&self, device: Opened) {
        self.edit(|record| {
            record.devices.retain(|kept| kept.native_id != device.native_id);
            record.devices.push(device);
        });
    }

    /// `owner` closed its device `native_id`, or lost it; answers whether
    /// the entry was `owner`'s. An entry another service took over stays.
    pub fn closed(&self, native_id: &str, owner: &str) -> bool {
        self.edit(|record| {
            let before = record.devices.len();
            record
                .devices
                .retain(|kept| !(kept.native_id == native_id && kept.owner == owner));
            record.devices.len() != before
        })
    }

    /// Whether `owner` still holds `native_id` in the record.
    pub fn owns(&self, native_id: &str, owner: &str) -> bool {
        self.edit(|record| {
            record
                .devices
                .iter()
                .any(|kept| kept.native_id == native_id && kept.owner == owner)
        })
    }

    /// `conversation` reached the service `owner`: the devices it has open
    /// become `owner`'s, and are answered. No other conversation's entry is
    /// touched.
    pub fn claim(&self, conversation: &str, owner: &str) -> Vec<Opened> {
        self.edit(|record| {
            let mut claimed = Vec::new();
            for kept in record.devices.iter_mut().filter(|kept| kept.conversation == conversation) {
                kept.owner = owner.to_owned();
                claimed.push(kept.clone());
            }
            claimed
        })
    }

    /// Removes every entry whose conversation `exists` denies, and answers
    /// them.
    pub fn forget_where_gone(&self, exists: impl Fn(&str) -> bool) -> Vec<Opened> {
        self.edit(|record| {
            let (kept, gone): (Vec<Opened>, Vec<Opened>) =
                record.devices.drain(..).partition(|device| exists(&device.conversation));
            record.devices = kept;
            gone
        })
    }

    /// The simulators Demi booted, and the devices open.
    #[cfg(feature = "testing")]
    pub fn snapshot(&self) -> (Vec<Entry>, Vec<Opened>) {
        self.edit(|record| (record.simulators.clone(), record.devices.clone()))
    }

    /// Runs `change` on the record as the file holds it now, under the
    /// lock, and writes the file when `change` changed it, before it
    /// answers. A write that fails is the Host log's.
    fn edit<T>(&self, change: impl FnOnce(&mut Record) -> T) -> T {
        let Some(directory) = &self.directory else {
            return change(&mut self.memory.lock().expect("the record is intact"));
        };
        if let Err(error) = std::fs::create_dir_all(directory) {
            tracing::error!("{} could not be made: {error}", directory.display());
        }
        // The lock is released when the file closes, with this function.
        let lock = File::options()
            .create(true)
            .write(true)
            .truncate(false)
            .open(directory.join(LOCK))
            .and_then(|lock| lock.lock().map(|()| lock));
        if let Err(error) = &lock {
            tracing::error!("{} could not be locked: {error}", directory.join(LOCK).display());
        }
        let path = directory.join(FILE);
        let mut record = read(&path);
        let before = record.clone();
        let answer = change(&mut record);
        if record != before
            && let Err(error) = write(&path, &record)
        {
            tracing::error!("{} could not be written: {error}", path.display());
        }
        drop(lock);
        answer
    }
}

/// The record in `path`; an empty one when the file is not there or does
/// not read, which is reported, and the next write replaces.
fn read(path: &Path) -> Record {
    match std::fs::read(path) {
        Ok(bytes) => match demi_shared_types::decode_slice::<Record>(&bytes) {
            Ok(record) => record,
            Err(error) => {
                tracing::error!("{} does not read, and is replaced: {error}", path.display());
                Record::default()
            }
        },
        Err(error) if error.kind() == io::ErrorKind::NotFound => Record::default(),
        Err(error) => {
            tracing::error!("{} could not be read: {error}", path.display());
            Record::default()
        }
    }
}

/// Writes `record` to `path` through a file of its own beside it, so a
/// program killed mid-write leaves the old record or the new one, and no
/// other process's write is the one it renames.
fn write(path: &Path, record: &Record) -> io::Result<()> {
    static WRITES: AtomicU64 = AtomicU64::new(0);
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let partial = path.with_extension(format!(
        "json.{}.{}.partial",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&partial, serde_json::to_vec(record).map_err(io::Error::other)?)?;
    std::fs::rename(&partial, path)
}
