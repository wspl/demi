//! The simulators Demi booted (`crates/command-package-mobile/src/devices.rs`), kept in
//! `<installation>/mobile/booted.json` across the program's ends: a
//! `demi-mobile` that stops, updates or is killed shuts nothing down, and
//! the next one gives each simulator still booted from Demi's boot the
//! hour without a watcher or a command. Each entry names the boot by its
//! mark (`driver::Simulator::boot`), so a simulator the user shut down and
//! booted again is the user's.

use std::{
    io,
    path::{Path, PathBuf},
    sync::Mutex,
};

use serde::{Deserialize, Serialize};

use demi_command_package_mobile_protocol::mobile::MobilePlatform;

/// The record's file in the installation's `mobile/`.
const FILE: &str = "booted.json";

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

/// The file's content.
#[derive(Debug, Default, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
struct Record {
    #[garde(dive)]
    simulators: Vec<Entry>,
}

/// The record, read once as the program starts and written at each change.
pub struct Booted {
    /// None for a program started without an installation, which keeps
    /// nothing across its end.
    path: Option<PathBuf>,
    entries: Mutex<Vec<Entry>>,
}

impl Booted {
    /// The record in `mobile` (the installation's `mobile/`), as the last
    /// program left it. A file that does not read is reported and replaced
    /// at the next change: what it named is the user's to shut down.
    pub fn load(mobile: Option<&Path>) -> Self {
        let path = mobile.map(|directory| directory.join(FILE));
        let entries = match &path {
            None => Vec::new(),
            Some(path) => match std::fs::read(path) {
                Ok(bytes) => match demi_shared_types::decode_slice::<Record>(&bytes) {
                    Ok(record) => record.simulators,
                    Err(error) => {
                        tracing::error!("{} does not read, and is replaced: {error}", path.display());
                        Vec::new()
                    }
                },
                Err(error) if error.kind() == io::ErrorKind::NotFound => Vec::new(),
                Err(error) => {
                    tracing::error!("{} could not be read: {error}", path.display());
                    Vec::new()
                }
            },
        };
        Self {
            path,
            entries: Mutex::new(entries),
        }
    }

    pub fn entries(&self) -> Vec<Entry> {
        self.entries.lock().expect("the record is intact").clone()
    }

    /// Whether Demi booted `native_id` of `platform` in its boot `boot`.
    pub fn holds(&self, platform: MobilePlatform, native_id: &str, boot: Option<u64>) -> bool {
        boot.is_some_and(|boot| {
            self.entries
                .lock()
                .expect("the record is intact")
                .iter()
                .any(|entry| entry.platform == platform && entry.native_id == native_id && entry.boot == boot)
        })
    }

    /// Demi booted `entry`'s simulator; an older boot of it leaves.
    pub fn add(&self, entry: Entry) {
        self.change(|entries| {
            entries.retain(|kept| !(kept.platform == entry.platform && kept.native_id == entry.native_id));
            entries.push(entry);
        });
    }

    /// `native_id` of `platform` is no longer Demi's to shut down.
    pub fn remove(&self, platform: MobilePlatform, native_id: &str) {
        self.change(|entries| entries.retain(|kept| !(kept.platform == platform && kept.native_id == native_id)));
    }

    /// Changes the entries and writes the file, as a whole, before it
    /// answers. A write that fails is the Host log's: the entry in memory
    /// still counts until the program ends.
    fn change(&self, edit: impl FnOnce(&mut Vec<Entry>)) {
        let mut entries = self.entries.lock().expect("the record is intact");
        let before = entries.clone();
        edit(&mut entries);
        if *entries == before {
            return;
        }
        let Some(path) = &self.path else {
            return;
        };
        let record = Record {
            simulators: entries.clone(),
        };
        if let Err(error) = write(path, &record) {
            tracing::error!("{} could not be written: {error}", path.display());
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
