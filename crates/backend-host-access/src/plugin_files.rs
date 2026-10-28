//! What the user's plugins reach on a Host.
//!
//! The module serves two things:
//!
//! - Reads of a conversation's primary Host, or of a command job's Host, in the
//!   form that never wakes it.
//! - The plugins' directories, which each job of the user's installs before it
//!   starts, once per runner connection.
//!
//! A plugin never writes a Host: it names the directories its user needs and
//! host access installs them.
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
use demi_runner_client::{DirectoryFile, HostSubscriber, Link, Look, LookAt, RemoteHost, WeakLink};
use bytes::{Bytes, BytesMut};
use futures_util::StreamExt;
use demi_host_interface::{ByteRange, FileKind, Host as _, HostError, HostErrorKind, HostFs as _};
use demi_runner_protocol::wire::FileHash;
use demi_plugin_interface::{EntryKind, HostDirectory, HostEntry, HostFile, HostRead, PluginId};
use demi_shared_types::{B64Bytes, ReadAt};
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
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
    /// The conversation works on another Host than the one the read was
    /// asked of.
    #[error("the conversation works on another Host now")]
    Moved,
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
    /// Reads `reads` on the conversation's primary Host, admitted as a look. A
    /// stopped Cloud or an offline device is not running and is not woken, the
    /// read is no activity, and a transition ends it instead of waiting for it.
    ///
    /// A path the Host cannot read answers as unreadable. The rest answer in
    /// the order of `reads`, with the Host's number before it read them
    /// (`crates/runner-host/src/subscriptions.rs`).
    pub async fn read_files(
        &self,
        id: &ConversationId,
        reads: &[HostRead],
        cancel: &CancellationToken,
    ) -> Result<(Vec<HostFile>, ReadAt), ReadFilesError> {
        self.read_files_on(id, None, reads, cancel).await
    }

    /// [`read_files`](Self::read_files), on the primary Host of `device`
    /// alone when one is named: [`ReadFilesError::Moved`] once the
    /// conversation works on another, so what a search of one Host finds is
    /// never read on another.
    pub async fn read_files_on(
        &self,
        id: &ConversationId,
        device: Option<&DeviceId>,
        reads: &[HostRead],
        cancel: &CancellationToken,
    ) -> Result<(Vec<HostFile>, ReadAt), ReadFilesError> {
        let access = self.admit_stream(id, None, Attention::Looks, cancel).await?;
        if device.is_some_and(|device| *device != access.device) {
            return Err(ReadFilesError::Moved);
        }
        let waits = Waits::until_ended(cancel, &access.open.ended);
        let found = waits.wait(read_host_files(&access.host.host, &self.blobs(), reads)).await?;
        Ok(found.map_err(HostAccessError::Host)?)
    }

    /// Reads `reads` for a command's job on `device`, the conversation's
    /// primary Host or an attached one, admitted as
    /// [`read_files`](Self::read_files) is; a relative path is read against
    /// `directory`, the job's, as that Host resolves it (`crates/plugin-interface/src/lib.rs`).
    pub async fn read_job_files(
        &self,
        id: &ConversationId,
        device: &DeviceId,
        directory: &str,
        reads: &[HostRead],
        cancel: &CancellationToken,
    ) -> Result<Vec<HostFile>, ReadFilesError> {
        let access = self.admit_stream(id, Some(device), Attention::Looks, cancel).await?;
        let paths = access.host.host.identity().paths;
        let reads: Vec<HostRead> = reads
            .iter()
            .map(|read| HostRead {
                path: paths.resolve(directory, &read.path),
                ..read.clone()
            })
            .collect();
        let waits = Waits::until_ended(cancel, &access.open.ended);
        let found = waits.wait(read_host_files(&access.host.host, &self.blobs(), &reads)).await?;
        Ok(found.map_err(HostAccessError::Host)?.0)
    }

    /// A holder of subscriptions on the conversation's primary Host
    /// (`crates/runner-host/src/subscriptions.rs`), and that Host's device, admitted as
    /// [`read_files`](Self::read_files) is: a stopped Cloud or an offline
    /// device is not running and is not woken. The holder keeps no
    /// admission, so it keeps no Cloud awake and no transition waits for it.
    pub async fn watch_files(
        &self,
        id: &ConversationId,
        cancel: &CancellationToken,
    ) -> Result<(DeviceId, HostSubscriber), ReadFilesError> {
        let access = self.admit_stream(id, None, Attention::Looks, cancel).await?;
        let subscriber = access
            .host
            .host
            .subscriber()
            .map_err(|error| ReadFilesError::from(HostAccessError::Host(error)))?;
        Ok((access.device, subscriber))
    }

    /// Brings `host`, admitted on `device` for a job, to the user's directory
    /// sets, once per runner connection and set.
    ///
    /// 1. It lists every plugin's directory on the Host with one request.
    /// 2. It removes what the sets no longer name with one more request.
    /// 3. It writes each directory that is missing with one request each.
    ///
    /// A failure fails the job.
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
        sync_directories(&host.host, home, &sets, &self.blobs()).await?;
        installs
            .synced
            .borrow_mut()
            .insert(device.clone(), (link.downgrade(), revision));
        Ok(())
    }
}

/// Brings the plugins' directories on `host` to `sets`: every plugin's
/// directory listed with one request, what no set names removed with one,
/// and each directory missing written with one, all at once.
async fn sync_directories(
    host: &RemoteHost,
    home: &str,
    sets: &DirectorySets,
    blobs: &UserBlobs,
) -> Result<(), HostError> {
    let paths = host.identity().paths;
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
        host.look(&listed).await?.0
    };
    let mut removed = Vec::new();
    let mut missing = Vec::new();
    for ((plugin, set), look) in sets.iter().zip(found) {
        let present: Vec<String> = match look {
            Look::Directory(entries) => entries.into_iter().map(|entry| entry.name).collect(),
            Look::Missing => Vec::new(),
            Look::Unreadable(error) => return Err(error),
            Look::File { .. } | Look::Other => {
                return Err(HostError::failed(
                    None,
                    format!("{} is not a directory", base(plugin)),
                ));
            }
        };
        let wanted: Vec<(String, &HostDirectory)> = set
            .iter()
            .map(|directory| (directory.host_name(), directory))
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
                .filter(|(name, _)| !present.contains(name))
                .map(|(name, directory)| (paths.join(&base(plugin), &name), directory)),
        );
    }
    if !removed.is_empty() {
        host.remove_all(&removed).await?;
    }
    let installs = missing
        .into_iter()
        .map(|(path, directory)| async move { install(host, blobs, &path, directory).await });
    futures_util::future::try_join_all(installs).await?;
    Ok(())
}

/// Writes `directory` at `path` with one request: its files read-only,
/// each keeping its executable bit, and every directory in it read-only
/// too, so a directory that exists is complete.
async fn install(
    host: &RemoteHost,
    blobs: &UserBlobs,
    path: &str,
    directory: &HostDirectory,
) -> Result<(), HostError> {
    let mut files = Vec::with_capacity(directory.files.len());
    for file in &directory.files {
        let bytes = blobs
            .get(&file.blob)
            .await
            .map_err(|error| HostError::failed(None, error.to_string()))?
            .ok_or_else(|| {
                HostError::failed(
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
    host.write_directory(path, files, 0o555).await
}

/// What each of `reads` finds on `host`, a read by content's file
/// ([`HostRead::content`]) as the bytes `blobs` hold of it, or one read of
/// them into `blobs` (`UserBlobs::fetch_once`): the Host hashes the file, and
/// its bytes travel only when they are new, so twenty commands that read the
/// same image send it once. The rest is [`look_files`].
pub async fn read_host_files(
    host: &RemoteHost,
    blobs: &UserBlobs,
    reads: &[HostRead],
) -> Result<(Vec<HostFile>, ReadAt), HostError> {
    let looks: Vec<HostRead> = reads
        .iter()
        .map(|read| if read.content { HostRead::new(read.path.clone(), 0) } else { read.clone() })
        .collect();
    let (mut files, read_at) = look_files(host, &looks).await?;
    let contents = reads.iter().zip(&files).enumerate().filter_map(|(index, (read, file))| match file {
        HostFile::File { size, .. } if read.content && *size <= read.limit => Some(async move {
            (index, content(host, blobs, &read.path, read.limit).await)
        }),
        _ => None,
    });
    for (index, read) in futures_util::future::join_all(contents).await {
        files[index] = match read {
            Ok(bytes) => HostFile::File {
                size: bytes.len() as u64,
                bytes: B64Bytes::new(bytes),
            },
            Err(message) => HostFile::Unreadable { message },
        };
    }
    Ok((files, read_at))
}

/// The bytes of the file at `path` on `host`, of at most `limit`: the ones
/// `blobs` hold under its hash, or a read of them into `blobs`.
async fn content(host: &RemoteHost, blobs: &UserBlobs, path: &str, limit: u64) -> Result<Bytes, String> {
    let (sha256, size) = match host.hash_file(path, limit).await.map_err(|error| error.message)? {
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
        let mut stream = host.read_stream(path, range).await?;
        let mut bytes = BytesMut::new();
        while let Some(chunk) = stream.next().await {
            bytes.extend_from_slice(&chunk?);
            if bytes.len() as u64 > size {
                return Err(HostError::interrupted("the runner sent more bytes than it was asked for"));
            }
        }
        Ok::<_, HostError>(bytes.freeze())
    };
    blobs.fetch_once(&sha256, read).await.map_err(|error| error.to_string())
}

/// What each of `reads` finds on `host`, with one request to the Host
/// (`crates/runner-host/src/lib.rs`).
///
/// A failure on the Host about a path answers for the path. Any other failure,
/// such as the connection's end, fails the read.
pub async fn look_files(host: &RemoteHost, reads: &[HostRead]) -> Result<(Vec<HostFile>, ReadAt), HostError> {
    let paths: Vec<LookAt> = reads
        .iter()
        .map(|read| LookAt {
            path: read.path.clone(),
            limit: read.limit,
        })
        .collect();
    let (found, read_at) = host.look(&paths).await?;
    let files = found
        .into_iter()
        .map(|look| match look {
            Look::Missing => HostFile::Missing,
            Look::Directory(entries) => HostFile::Directory {
                entries: entries
                    .into_iter()
                    .map(|entry| HostEntry {
                        name: entry.name,
                        kind: entry_kind(entry.kind),
                    })
                    .collect(),
            },
            Look::File { bytes, size } => HostFile::File {
                bytes: B64Bytes::new(bytes),
                size,
            },
            Look::Other => HostFile::Other,
            Look::Unreadable(error) => HostFile::Unreadable {
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

/// Whether `error` says the Host is not reachable now.
fn not_running(error: &HostError) -> bool {
    matches!(
        error.kind,
        HostErrorKind::Offline | HostErrorKind::Unavailable
    )
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
