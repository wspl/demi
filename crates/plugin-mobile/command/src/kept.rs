//! What the service keeps across its own ends
//! (`crates/plugin-mobile/command/src/devices.rs`), in
//! `<installation>/mobile/kept.json`:
//!
//! - the simulators Demi booted, so a simulator is shut down with its
//!   device only when Demi booted it. Each names the boot by its mark
//!   (`driver::Simulator::boot`), so a simulator the user shut down and
//!   booted again is the user's.
//! - the devices open, which conversation has each and under which handle,
//!   so a service that starts again takes each back under the same handle.
//!
//! A `demi-mobile` that stops, updates or is killed shuts nothing down and
//! forgets nothing; a device leaves the record when it closes or is lost.

use std::{
    io,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};

use demi_command_package_mobile_protocol::mobile::{DeviceId, MobilePlatform};

/// The record's file in the installation's `mobile/`.
const FILE: &str = "kept.json";

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

/// A device a conversation has open. Unknown fields are not refused: a file
/// an earlier version wrote names a service that held the device (`owner`),
/// which nothing reads now.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct Opened {
    #[garde(skip)]
    pub handle: DeviceId,
    #[garde(length(min = 1))]
    pub conversation: String,
    #[garde(skip)]
    pub platform: MobilePlatform,
    #[garde(length(min = 1))]
    pub native_id: String,
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

/// The record, read once as the program starts and written at each change.
pub struct Kept {
    /// None for a program started without an installation, which keeps
    /// nothing across its end.
    path: Option<PathBuf>,
    record: Mutex<Record>,
}

impl Kept {
    /// The record in `mobile` (the installation's `mobile/`), as the last
    /// program left it. A file that does not read is reported and replaced
    /// at the next change: what it named is the user's to shut down, and the
    /// devices it named are not open any more.
    pub fn load(mobile: Option<&Path>) -> Self {
        let path = mobile.map(|directory| directory.join(FILE));
        let record = match &path {
            None => Record::default(),
            Some(path) => match std::fs::read(path) {
                Ok(bytes) => match demi_shared_types::decode_slice::<Record>(&bytes) {
                    Ok(record) => record,
                    Err(error) => {
                        tracing::error!(event = "mobile_state.failed", op = "read_replaced", path = %path.display(), %error, "the record of the devices does not read, and is replaced");
                        Record::default()
                    }
                },
                Err(error) if error.kind() == io::ErrorKind::NotFound => Record::default(),
                Err(error) => {
                    tracing::error!(event = "mobile_state.failed", op = "read", path = %path.display(), %error, "the record of the devices could not be read");
                    Record::default()
                }
            },
        };
        Self {
            path,
            record: Mutex::new(record),
        }
    }

    /// Whether Demi booted `native_id` of `platform` in its boot `boot`.
    pub fn holds(&self, platform: MobilePlatform, native_id: &str, boot: Option<u64>) -> bool {
        boot.is_some_and(|boot| {
            self.record
                .lock()
                .expect("the record is intact")
                .simulators
                .iter()
                .any(|entry| entry.platform == platform && entry.native_id == native_id && entry.boot == boot)
        })
    }

    /// Demi booted `entry`'s simulator; an older boot of it leaves.
    pub fn booted(&self, entry: Entry) {
        self.change(|record| {
            record
                .simulators
                .retain(|kept| !(kept.platform == entry.platform && kept.native_id == entry.native_id));
            record.simulators.push(entry);
        });
    }

    /// `native_id` of `platform` is no longer Demi's to shut down.
    pub fn unbooted(&self, platform: MobilePlatform, native_id: &str) {
        self.change(|record| {
            record
                .simulators
                .retain(|kept| !(kept.platform == platform && kept.native_id == native_id));
        });
    }

    /// The devices of `conversation` the last program left open.
    pub fn claim(&self, conversation: &str) -> Vec<Opened> {
        let record = self.record.lock().expect("the record is intact");
        record.devices.iter().filter(|kept| kept.conversation == conversation).cloned().collect()
    }

    /// A conversation opened `device`.
    pub fn opened(&self, device: Opened) {
        self.change(|record| {
            record.devices.retain(|kept| kept.native_id != device.native_id);
            record.devices.push(device);
        });
    }

    /// The device `native_id` closed or is lost.
    pub fn closed(&self, native_id: &str) {
        self.change(|record| record.devices.retain(|kept| kept.native_id != native_id));
    }

    /// Removes every entry whose conversation `exists` denies, and answers
    /// them.
    pub fn forget_where_gone(&self, exists: impl Fn(&str) -> bool) -> Vec<Opened> {
        let mut gone = Vec::new();
        self.change(|record| {
            let (kept, left): (Vec<Opened>, Vec<Opened>) =
                record.devices.drain(..).partition(|device| exists(&device.conversation));
            record.devices = kept;
            gone = left;
        });
        gone
    }

    /// Changes the record and writes the file, as a whole, before it
    /// answers. A write that fails is the Host log's: the change in memory
    /// still counts until the program ends.
    fn change(&self, edit: impl FnOnce(&mut Record)) {
        let mut record = self.record.lock().expect("the record is intact");
        let before = record.clone();
        edit(&mut record);
        if *record == before {
            return;
        }
        let Some(path) = &self.path else {
            return;
        };
        if let Err(error) = write(path, &record) {
            tracing::error!(event = "mobile_state.failed", op = "write", path = %path.display(), %error, "the record of the devices could not be written");
        }
    }
}

/// Writes `record` to `path` through a file beside it, so a program killed
/// mid-write leaves the old record or the new one.
fn write(path: &Path, record: &Record) -> io::Result<()> {
    if let Some(directory) = path.parent() {
        std::fs::create_dir_all(directory)?;
    }
    let partial = path.with_extension("json.partial");
    std::fs::write(&partial, serde_json::to_vec(record).map_err(io::Error::other)?)?;
    std::fs::rename(&partial, path)
}
