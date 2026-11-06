//! What the user's plugins reach on a device.
//!
//! The module serves two things:
//!
//! - Reads of a conversation's primary device, or of a command job's device, in the
//!   form that never wakes it.
//! - The plugins' directories, which each job of the user's installs before it
//!   starts, once per runner connection.
//!
//! A plugin never writes a device: it names the directories its user needs and
//! device access installs them.
//!
//! - A directory that exists is complete. The runner writes it beside the final
//!   one, read-only, and renames it into place, so a crash leaves at most a
//!   temporary directory, which the next installation removes.
//! - The same installation removes the plugin's directories the user's set no
//!   longer names. A command still reading one fails as with any file removed
//!   underneath it, and the model was already told the new paths.
//! - A new connection checks again, so a Cloud's new boot is covered and a
//!   directory the user deleted by hand comes back.
//! - A job never waits for a directory it does not need, and a conversation
//!   that runs no job installs nothing, so one that only talks never wakes its
//!   Cloud for a plugin's files.

use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap};

use demi_backend_blobs::blobs::UserBlobs;
use demi_runner_client::{DirectoryFile, DeviceSubscriber, Link, Look, LookAt, RemoteDevice, WeakLink};
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use demi_device_interface::{ByteRange, FileKind, Device as _, DeviceError, DeviceErrorKind, DeviceFs as _};
use demi_runner_protocol::wire::FileHash;
use demi_plugin_interface::{DeviceDirectory, PluginId};
use demi_device_interface::{EntryKind, DeviceEntry, DeviceFile, DeviceRead};
use demi_shared_types::{B64Bytes, ReadAt};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, WorkspaceId};
use tokio_util::sync::CancellationToken;

use crate::DeviceShard;
use crate::access::{Attention, ConversationDevice, DeviceAccessError, Refusal, Waits};

/// The device directories of the user's plugins: every plugin with its set,
/// an empty one for a plugin the user has off, whose directories a device
/// loses at its next installation.
pub type DirectorySets = Vec<(PluginId, Vec<DeviceDirectory>)>;

/// What the shard remembers of its users' devices' directories.
#[derive(Default)]
pub struct PluginInstalls {
    /// Per device, the connection its device was last brought to the user's
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

/// Why a read of a conversation's device did not answer.
#[derive(Debug, thiserror::Error)]
pub enum ReadFilesError {
    /// The device is not running, and a read never wakes it.
    #[error("the conversation's device is not running")]
    NotRunning,
    /// The conversation works on another device than the one the read was
    /// asked of.
    #[error("the conversation works on another device now")]
    Moved,
    #[error(transparent)]
    Access(DeviceAccessError),
}

impl From<DeviceAccessError> for ReadFilesError {
    fn from(error: DeviceAccessError) -> Self {
        match error {
            DeviceAccessError::Refused(Refusal::Stopped) => Self::NotRunning,
            DeviceAccessError::Device(error) if not_running(&error) => Self::NotRunning,
            error => Self::Access(error),
        }
    }
}

impl dyn DeviceShard + '_ {
    /// Reads `reads` on the conversation's primary device, admitted as a look. A
    /// stopped Cloud or an offline device is not running and is not woken, the
    /// read is no activity, and a transition ends it instead of waiting for it.
    ///
    /// A path the device cannot read answers as unreadable. The rest answer in
    /// the order of `reads`, with the device's number before it read them
    /// (`crates/runner-device/src/subscriptions.rs`).
    pub async fn read_files(
        &self,
        id: &ConversationId,
        reads: &[DeviceRead],
        cancel: &CancellationToken,
    ) -> Result<(Vec<DeviceFile>, ReadAt), ReadFilesError> {
        self.read_files_on(id, None, reads, cancel).await
    }

    /// [`read_files`](Self::read_files), on the device of `device` alone when
    /// one is named, the primary one or one a live subagent works on:
    /// [`ReadFilesError::Moved`] once the conversation works on neither, so
    /// what a search of one device finds is never read on another.
    pub async fn read_files_on(
        &self,
        id: &ConversationId,
        device: Option<&DeviceId>,
        reads: &[DeviceRead],
        cancel: &CancellationToken,
    ) -> Result<(Vec<DeviceFile>, ReadAt), ReadFilesError> {
        if let Some(device) = device
            && !self.works_on(id, device).await?
        {
            return Err(ReadFilesError::Moved);
        }
        let access = self.admit_stream(id, device, Attention::Looks, cancel).await?;
        let waits = Waits::until_ended(cancel, &access.open.ended);
        let found = waits.wait(read_device_files(&access.admitted.device, &self.blobs(), reads)).await?;
        Ok(found.map_err(DeviceAccessError::Device)?)
    }

    /// Reads `reads` for a command's job on `device`, the conversation's
    /// primary device or an attached one, admitted as
    /// [`read_files`](Self::read_files) is; a relative path is read against
    /// `directory`, the job's, as that device resolves it (`crates/plugin-interface/src/lib.rs`).
    pub async fn read_job_files(
        &self,
        id: &ConversationId,
        device: &DeviceId,
        directory: &str,
        reads: &[DeviceRead],
        cancel: &CancellationToken,
    ) -> Result<Vec<DeviceFile>, ReadFilesError> {
        let access = self.admit_stream(id, Some(device), Attention::Looks, cancel).await?;
        let paths = access.admitted.device.identity().paths;
        let reads: Vec<DeviceRead> = reads
            .iter()
            .map(|read| DeviceRead {
                path: paths.resolve(directory, &read.path),
                ..read.clone()
            })
            .collect();
        let waits = Waits::until_ended(cancel, &access.open.ended);
        let found = waits.wait(read_device_files(&access.admitted.device, &self.blobs(), &reads)).await?;
        Ok(found.map_err(DeviceAccessError::Device)?.0)
    }

    /// A holder of subscriptions on the device of `device`
    /// (`crates/runner-device/src/subscriptions.rs`), the conversation's primary
    /// one or one a live subagent works on, admitted as
    /// [`read_files`](Self::read_files) is: a stopped Cloud or an offline
    /// device is not running and is not woken, and a device the conversation
    /// works on no longer is [`ReadFilesError::Moved`]. The holder keeps no
    /// admission, so it keeps no Cloud awake and no transition waits for it.
    pub async fn watch_files(
        &self,
        id: &ConversationId,
        device: &DeviceId,
        cancel: &CancellationToken,
    ) -> Result<(DeviceId, DeviceSubscriber), ReadFilesError> {
        if !self.works_on(id, device).await? {
            return Err(ReadFilesError::Moved);
        }
        let access = self.admit_stream(id, Some(device), Attention::Looks, cancel).await?;
        let subscriber = access
            .admitted
            .device
            .subscriber()
            .map_err(|error| ReadFilesError::from(DeviceAccessError::Device(error)))?;
        Ok((access.device, subscriber))
    }

    /// Brings `device`, admitted as `admitted` for a job of `conversation`, to the
    /// user's directory sets, once per runner connection, set and workspace.
    ///
    /// 1. It lists every plugin's directory on the device with one request.
    /// 2. It removes what the sets no longer name with one more request.
    /// 3. It writes each directory the job's conversation needs that is
    ///    missing with one request each: those of every job and those of the
    ///    conversation's workspace. Another workspace's stay as they are.
    ///
    /// A failure fails the job.
    pub(crate) async fn install_directories(
        &self,
        conversation: &ConversationId,
        device: &DeviceId,
        admitted: &ConversationDevice,
    ) -> Result<(), DeviceError> {
        let Some(link) = self.devices().link(device) else {
            // The job's own start reports the connection's end.
            return Ok(());
        };
        let sets = self
            .directory_sets()
            .await
            .map_err(|error| DeviceError::failed(None, error))?;
        let workspace = self
            .control()
            .conversation(conversation.clone())
            .await
            .map_err(|error| DeviceError::failed(None, error.to_string()))?
            .map(|record| record.workspace);
        let revision = format!(
            "{}for {}",
            revision_of(&sets),
            workspace.as_ref().map_or("", WorkspaceId::as_str)
        );
        let installs = self.plugin_installs();
        if installs.synced(device, &link, &revision) {
            return Ok(());
        }
        let _turn = installs.turn.lock().await;
        if installs.synced(device, &link, &revision) {
            return Ok(());
        }
        let home = admitted.home.as_deref().ok_or_else(|| {
            DeviceError::failed(
                None,
                "the device reported no home directory for the plugins' files",
            )
        })?;
        sync_directories(&admitted.device, home, &sets, workspace.as_ref(), &self.blobs()).await?;
        installs
            .synced
            .borrow_mut()
            .insert(device.clone(), (link.downgrade(), revision));
        Ok(())
    }
}

/// Brings the plugins' directories on `device` to `sets`: every plugin's
/// directory listed with one request, what no set names removed with one,
/// and each directory missing that a job of `workspace` needs, every job's
/// and the workspace's own, written with one, all at once.
async fn sync_directories(
    device: &RemoteDevice,
    home: &str,
    sets: &DirectorySets,
    workspace: Option<&WorkspaceId>,
    blobs: &UserBlobs,
) -> Result<(), DeviceError> {
    let paths = device.identity().paths;
    let base = |plugin: &PluginId| paths.join(home, &format!(".demi/plugins/{plugin}"));
    let listed: Vec<LookAt> = sets
        .iter()
        .map(|(plugin, _)| LookAt {
            path: base(plugin),
            limit: 0,
        })
        .collect();
    let found = if listed.is_empty() {
        Vec::new()
    } else {
        device.look(&listed).await?.0
    };
    let mut removed = Vec::new();
    let mut missing = Vec::new();
    for ((plugin, set), look) in sets.iter().zip(found) {
        let present: Vec<String> = match look {
            Look::Directory(entries) => entries.into_iter().map(|entry| entry.name).collect(),
            Look::Missing => Vec::new(),
            Look::Unreadable(error) => return Err(error),
            Look::File { .. } | Look::Other => {
                return Err(DeviceError::failed(
                    None,
                    format!("{} is not a directory", base(plugin)),
                ));
            }
        };
        let wanted: Vec<(String, &DeviceDirectory)> = set
            .iter()
            .map(|directory| (directory.device_name(), directory))
            .collect();
        let names: BTreeSet<&str> = wanted.iter().map(|(name, _)| name.as_str()).collect();
        removed.extend(
            present
                .iter()
                .filter(|name| !names.contains(name.as_str()))
                .map(|name| paths.join(&base(plugin), name)),
        );
        missing.extend(
            wanted
                .into_iter()
                .filter(|(_, directory)| directory.workspace.is_none() || directory.workspace.as_ref() == workspace)
                .filter(|(name, _)| !present.contains(name))
                .map(|(name, directory)| (paths.join(&base(plugin), &name), directory)),
        );
    }
    if !removed.is_empty() {
        device.remove_all(&removed).await?;
    }
    let installs = missing
        .into_iter()
        .map(|(path, directory)| async move { install(device, blobs, &path, directory).await });
    futures_util::future::try_join_all(installs).await?;
    Ok(())
}

/// Writes `directory` at `path` with one request: its files read-only,
/// each keeping its executable bit, and every directory in it read-only
/// too, so a directory that exists is complete.
async fn install(
    device: &RemoteDevice,
    blobs: &UserBlobs,
    path: &str,
    directory: &DeviceDirectory,
) -> Result<(), DeviceError> {
    let mut files = Vec::with_capacity(directory.files.len());
    for file in &directory.files {
        let bytes = blobs
            .get(&file.blob)
            .await
            .map_err(|error| DeviceError::failed(None, error.to_string()))?
            .ok_or_else(|| {
                DeviceError::failed(
                    None,
                    format!("the file {} of {path} is not stored", file.path),
                )
            })?;
        files.push(DirectoryFile {
            path: file.path.clone(),
            mode: if file.executable { 0o555 } else { 0o444 },
            bytes,
        });
    }
    device.write_directory(path, files, 0o555).await
}

/// What each of `reads` finds on `device`, a read by content's file
/// ([`DeviceRead::content`]) as the bytes `blobs` hold of it, or one read of
/// them into `blobs` (`UserBlobs::fetch_once`): the device hashes the file, and
/// its bytes travel only when they are new, so twenty commands that read the
/// same image send it once. The rest is [`look_files`].
pub async fn read_device_files(
    device: &RemoteDevice,
    blobs: &UserBlobs,
    reads: &[DeviceRead],
) -> Result<(Vec<DeviceFile>, ReadAt), DeviceError> {
    let looks: Vec<DeviceRead> = reads
        .iter()
        .map(|read| if read.content { DeviceRead::new(read.path.clone(), 0) } else { read.clone() })
        .collect();
    let (mut files, read_at) = look_files(device, &looks).await?;
    let contents = reads.iter().zip(&files).enumerate().filter_map(|(index, (read, file))| match file {
        DeviceFile::File { size, .. } if read.content && *size <= read.limit => Some(async move {
            (index, content(device, blobs, &read.path, read.limit).await)
        }),
        _ => None,
    });
    for (index, read) in futures_util::future::join_all(contents).await {
        files[index] = match read {
            Ok(bytes) => DeviceFile::File {
                size: bytes.len() as u64,
                bytes: B64Bytes::new(bytes),
            },
            Err(message) => DeviceFile::Unreadable { message },
        };
    }
    Ok((files, read_at))
}

/// The bytes of the file at `path` on `device`, of at most `limit`: the ones
/// `blobs` hold under its hash, or a read of them into `blobs`.
async fn content(device: &RemoteDevice, blobs: &UserBlobs, path: &str, limit: u64) -> Result<Bytes, String> {
    let (sha256, size) = match device.hash_file(path, limit).await.map_err(|error| error.message)? {
        FileHash::Hashed { sha256, size, .. } => (sha256, size),
        FileHash::TooLarge { size } => return Err(format!("the file grew to {size} bytes while it was read")),
    };
    // Only the bytes hashed: a file replaced since by a larger one is read
    // no further, and fails as changed.
    let read = || async move {
        let range = ByteRange {
            offset: 0,
            length: Some(size),
        };
        let mut stream = device.read_stream(path, range).await?;
        let mut bytes = BytesMut::new();
        while let Some(chunk) = stream.next().await {
            bytes.extend_from_slice(&chunk?);
            if bytes.len() as u64 > size {
                return Err(DeviceError::interrupted("the runner sent more bytes than it was asked for"));
            }
        }
        Ok::<_, DeviceError>(bytes.freeze())
    };
    blobs.fetch_once(&sha256, read).await.map_err(|error| error.to_string())
}

/// What each of `reads` finds on `device`, with one request to the device
/// (`crates/runner-device/src/lib.rs`).
///
/// A failure on the device about a path answers for the path. Any other failure,
/// such as the connection's end, fails the read.
pub async fn look_files(device: &RemoteDevice, reads: &[DeviceRead]) -> Result<(Vec<DeviceFile>, ReadAt), DeviceError> {
    let paths: Vec<LookAt> = reads
        .iter()
        .map(|read| LookAt {
            path: read.path.clone(),
            limit: read.limit,
        })
        .collect();
    let (found, read_at) = device.look(&paths).await?;
    let files = found
        .into_iter()
        .map(|look| match look {
            Look::Missing => DeviceFile::Missing,
            Look::Directory(entries) => DeviceFile::Directory {
                entries: entries
                    .into_iter()
                    .map(|entry| DeviceEntry {
                        name: entry.name,
                        kind: entry_kind(entry.kind),
                    })
                    .collect(),
            },
            Look::File { bytes, size } => DeviceFile::File {
                bytes: B64Bytes::new(bytes),
                size,
            },
            Look::Other => DeviceFile::Other,
            Look::Unreadable(error) => DeviceFile::Unreadable {
                message: error.message,
            },
        })
        .collect();
    Ok((files, read_at))
}

fn entry_kind(kind: FileKind) -> EntryKind {
    match kind {
        FileKind::File => EntryKind::File,
        FileKind::Directory => EntryKind::Directory,
        FileKind::Symlink => EntryKind::Symlink,
        FileKind::CharacterDevice | FileKind::Fifo | FileKind::Other => EntryKind::Other,
    }
}

/// Whether `error` says the device is not reachable now.
fn not_running(error: &DeviceError) -> bool {
    matches!(
        error.kind,
        DeviceErrorKind::Offline | DeviceErrorKind::Unavailable
    )
}

/// The revision of the user's sets, which a device is brought to once per
/// connection: each plugin with the names its device directory holds and the
/// workspace each is for, since a directory that moves to another workspace
/// keeps its name.
fn revision_of(sets: &DirectorySets) -> String {
    let mut revision = String::new();
    for (plugin, set) in sets {
        revision.push_str(plugin.as_str());
        for directory in set {
            revision.push(' ');
            revision.push_str(&directory.device_name());
            if let Some(workspace) = &directory.workspace {
                revision.push('@');
                revision.push_str(workspace.as_str());
            }
        }
        revision.push('\n');
    }
    revision
}
