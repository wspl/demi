//! What the user's plugins reach on a Host (`plugins.md` § Host
//! directories, § Reading a conversation's files): reads of a
//! conversation's primary Host in the form that never wakes it, and the
//! plugins' directories, which each job of the user's installs before it
//! starts, once per runner connection.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

use bytes::Bytes;
use demi_backend_remote_host::{Link, RemoteHost, WeakLink};
use demi_host_interface::{
    ByteRange, FileContents, FileKind, HostError, HostErrorKind, HostFs, MkdirOptions, RmOptions,
    WriteOptions,
};
use demi_plugin_interface::{EntryKind, HostDirectory, HostEntry, HostFile, HostRead, PluginId};
use demi_shared_types::B64Bytes;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use futures_util::TryStreamExt;
use tokio_util::sync::CancellationToken;

use crate::HostShard;
use crate::access::{Attention, ConversationHost, HostAccessError, Refusal, Waits};

/// The Host directories of the user's plugins: every plugin with its set,
/// an empty one for a plugin the user has off, whose directories a Host
/// loses at its next installation.
pub type DirectorySets = Vec<(PluginId, Vec<HostDirectory>)>;

/// What the shard remembers of its users' Hosts' directories.
#[derive(Default)]
pub struct PluginInstalls {
    /// Per device, the connection its Host was last brought to the user's
    /// set on, and that set's revision; a new connection checks again.
    synced: RefCell<HashMap<DeviceId, (WeakLink, String)>>,
    /// One installation at a time, so two never share a temporary
    /// directory.
    turn: tokio::sync::Mutex<()>,
}

impl PluginInstalls {
    fn synced(&self, device: &DeviceId, link: &Link, revision: &str) -> bool {
        self.synced
            .borrow()
            .get(device)
            .is_some_and(|(synced, at)| synced.is(link) && at == revision)
    }
}

/// Why a read of a conversation's Host did not answer.
#[derive(Debug, thiserror::Error)]
pub enum ReadFilesError {
    /// The Host is not running, and a read never wakes it.
    #[error("the conversation's Host is not running")]
    NotRunning,
    #[error(transparent)]
    Access(HostAccessError),
}

impl From<HostAccessError> for ReadFilesError {
    fn from(error: HostAccessError) -> Self {
        match error {
            HostAccessError::Refused(Refusal::Stopped) => Self::NotRunning,
            HostAccessError::Host(error) if not_running(&error) => Self::NotRunning,
            error => Self::Access(error),
        }
    }
}

impl dyn HostShard + '_ {
    /// Reads `reads` on the conversation's primary Host, admitted as a look:
    /// a stopped Cloud or an offline device is not running and is not
    /// woken, the read is no activity, and a transition ends it instead of
    /// waiting for it. A path the Host cannot read answers as unreadable;
    /// the rest answer in the order of `reads`.
    pub async fn read_files(
        &self,
        id: &ConversationId,
        reads: &[HostRead],
        cancel: &CancellationToken,
    ) -> Result<Vec<HostFile>, ReadFilesError> {
        let access = self.admit_stream(id, Attention::Looks, cancel).await?;
        let mut files = Vec::with_capacity(reads.len());
        for read in reads {
            let waits = Waits {
                cancel,
                ended: Some(&access.open.ended),
            };
            let found = waits.wait(look(&access.host.host, read)).await?;
            files.push(found.map_err(HostAccessError::Host)?);
        }
        Ok(files)
    }

    /// Brings `host`, admitted on `device` for a job, to the user's
    /// directory sets: once per runner connection and set, it lists each
    /// plugin's directory on the Host, installs what is missing and
    /// removes what the set no longer names. A failure fails the job.
    pub(crate) async fn install_directories(
        &self,
        device: &DeviceId,
        host: &ConversationHost,
    ) -> Result<(), HostError> {
        let Some(link) = self.devices().link(device) else {
            // The job's own start reports the connection's end.
            return Ok(());
        };
        let sets = self
            .directory_sets()
            .await
            .map_err(|error| HostError::failed(None, error))?;
        let revision = revision_of(&sets);
        let installs = self.plugin_installs();
        if installs.synced(device, &link, &revision) {
            return Ok(());
        }
        let _turn = installs.turn.lock().await;
        if installs.synced(device, &link, &revision) {
            return Ok(());
        }
        let home = host.home.as_deref().ok_or_else(|| {
            HostError::failed(
                None,
                "the Host reported no home directory for the plugins' files",
            )
        })?;
        for (plugin, set) in &sets {
            self.sync_plugin(&host.host, home, plugin, set).await?;
        }
        installs
            .synced
            .borrow_mut()
            .insert(device.clone(), (link.downgrade(), revision));
        Ok(())
    }

    /// Brings `plugin`'s directory on the Host to `set`.
    async fn sync_plugin(
        &self,
        host: &RemoteHost,
        home: &str,
        plugin: &PluginId,
        set: &[HostDirectory],
    ) -> Result<(), HostError> {
        let base = format!("{home}/.demi/plugins/{plugin}");
        let present: Vec<String> = match HostFs::read_dir(host, &base).await {
            Ok(entries) => entries.into_iter().map(|entry| entry.name).collect(),
            Err(error) if missing(&error) => Vec::new(),
            Err(error) => return Err(error),
        };
        let wanted: Vec<(String, &HostDirectory)> = set
            .iter()
            .map(|directory| (directory.host_name(), directory))
            .collect();
        let names: BTreeSet<&str> = wanted.iter().map(|(name, _)| name.as_str()).collect();
        for name in present.iter().filter(|name| !names.contains(name.as_str())) {
            remove(host, &format!("{base}/{name}")).await?;
        }
        for (name, directory) in &wanted {
            if !present.contains(name) {
                self.install(host, &base, name, directory).await?;
            }
        }
        Ok(())
    }

    /// Writes `directory` into a temporary directory beside its final one,
    /// makes what it holds read-only, keeping each file's executable bit,
    /// renames it into place and makes it read-only too, so a directory
    /// that exists is complete.
    async fn install(
        &self,
        host: &RemoteHost,
        base: &str,
        name: &str,
        directory: &HostDirectory,
    ) -> Result<(), HostError> {
        let partial = format!("{base}/.{name}.partial");
        remove(host, &partial).await?;
        HostFs::mkdir(host, &partial, MkdirOptions { recursive: true }).await?;
        let blobs = self.blobs();
        let mut directories = BTreeSet::new();
        for file in &directory.files {
            let bytes = blobs
                .get(&file.blob)
                .await
                .map_err(|error| HostError::failed(None, error.to_string()))?
                .ok_or_else(|| {
                    HostError::failed(
                        None,
                        format!("the file {} of {name} is not stored", file.path),
                    )
                })?;
            let path = format!("{partial}/{}", file.path);
            let contents = FileContents::Bytes(bytes);
            let options = WriteOptions {
                create_parents: true,
            };
            HostFs::write_file(host, &path, contents, options).await?;
            let mode = if file.executable { 0o555 } else { 0o444 };
            HostFs::chmod(host, &path, mode).await?;
            let mut parent = file.path.as_str();
            while let Some((above, _)) = parent.rsplit_once('/') {
                directories.insert(format!("{partial}/{above}"));
                parent = above;
            }
        }
        // Deepest first, so each is still writable while its entries change.
        for path in directories.iter().rev() {
            HostFs::chmod(host, path, 0o555).await?;
        }
        // Renamed while it is writable, which some systems ask of a
        // directory that moves; complete either way.
        let installed = format!("{base}/{name}");
        HostFs::mv(host, &partial, &installed).await?;
        HostFs::chmod(host, &installed, 0o555).await
    }
}

/// What `read` finds on `host`: a host failure that is not about the path
/// fails the whole read.
async fn look(host: &RemoteHost, read: &HostRead) -> Result<HostFile, HostError> {
    let stat = match HostFs::stat(host, &read.path).await {
        Ok(stat) => stat,
        Err(error) if missing(&error) => return Ok(HostFile::Missing),
        Err(error) => return unreadable(error),
    };
    match stat.kind {
        FileKind::Directory => match HostFs::read_dir(host, &read.path).await {
            Ok(entries) => Ok(HostFile::Directory {
                entries: entries
                    .into_iter()
                    .map(|entry| HostEntry {
                        name: entry.name,
                        kind: entry_kind(entry.kind),
                    })
                    .collect(),
            }),
            Err(error) => unreadable(error),
        },
        FileKind::File => {
            let range = ByteRange {
                offset: 0,
                length: Some(read.limit),
            };
            let stream = match HostFs::read_stream(host, &read.path, range).await {
                Ok(stream) => stream,
                Err(error) => return unreadable(error),
            };
            let chunks: Vec<Bytes> = stream.try_collect().await?;
            Ok(HostFile::File {
                bytes: B64Bytes::new(chunks.concat()),
                size: stat.size,
            })
        }
        _ => Ok(HostFile::Other),
    }
}

/// A failure on the Host about the path answers for the path; any other
/// failure, such as the connection's end, fails the read.
fn unreadable(error: HostError) -> Result<HostFile, HostError> {
    match error.kind {
        HostErrorKind::Failed { .. } => Ok(HostFile::Unreadable {
            message: error.message,
        }),
        _ => Err(error),
    }
}

fn entry_kind(kind: FileKind) -> EntryKind {
    match kind {
        FileKind::File => EntryKind::File,
        FileKind::Directory => EntryKind::Directory,
        FileKind::Symlink => EntryKind::Symlink,
        FileKind::CharacterDevice | FileKind::Fifo | FileKind::Other => EntryKind::Other,
    }
}

/// Whether `error` says the path, or a directory on its way, does not
/// exist.
fn missing(error: &HostError) -> bool {
    matches!(error.code(), Some("ENOENT" | "ENOTDIR"))
}

/// Whether `error` says the Host is not reachable now.
fn not_running(error: &HostError) -> bool {
    matches!(
        error.kind,
        HostErrorKind::Offline | HostErrorKind::Unavailable
    )
}

/// Removes `path`, which an installation made read-only: each directory
/// in it is made writable first, so its entries can go.
async fn remove(host: &RemoteHost, path: &str) -> Result<(), HostError> {
    let stat = match HostFs::lstat(host, path).await {
        Ok(stat) => stat,
        Err(error) if missing(&error) => return Ok(()),
        Err(error) => return Err(error),
    };
    if stat.kind == FileKind::Directory {
        let mut pending = vec![path.to_owned()];
        while let Some(directory) = pending.pop() {
            HostFs::chmod(host, &directory, 0o755).await?;
            for entry in HostFs::read_dir(host, &directory).await? {
                if entry.kind == FileKind::Directory {
                    pending.push(format!("{directory}/{}", entry.name));
                }
            }
        }
    }
    let options = RmOptions {
        recursive: true,
        force: true,
    };
    HostFs::rm(host, path, options).await
}

/// The revision of the user's sets, which a Host is brought to once per
/// connection: each plugin with the names its Host directory holds.
fn revision_of(sets: &DirectorySets) -> String {
    let mut revision = String::new();
    for (plugin, set) in sets {
        revision.push_str(plugin.as_str());
        for directory in set {
            revision.push(' ');
            revision.push_str(&directory.host_name());
        }
        revision.push('\n');
    }
    revision
}
