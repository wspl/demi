//! The product's `demi attachment` group: `upload` and `download` move files
//! between the Host and the conversation's attachments.
//!
//! `upload` copies files of the Host into the conversation as attachments `a1`,
//! `a2`, …, which the agent's messages then deliver as `<attachment id="a3" />`
//! blocks.
//!
//! 1. The runner of the invocation's Host, the conversation's primary Host or
//!    an attached one a `demi host shell` job runs on, hashes each file beside
//!    the invocation, through the conversation's host access. It refuses one
//!    over 25 MiB before reading a byte.
//! 2. A blob the conversation owner's namespace holds already is not read
//!    again. Only its first 64 KiB come from the object store, for its media
//!    type.
//! 3. Otherwise the runner streams the file's bytes to the backend through a
//!    pipe (`crates/runner-host/src/files.rs`), and the backend stores the blob
//!    in that namespace.
//! 4. The backend reads the media type from the bytes as it does for the user's
//!    uploads, and stores the record under the next number of the
//!    conversation's `attachment` sequence, with the file it came from: the
//!    invocation's device and the file's full path as that Host writes it,
//!    which a card of a file that is no image or video opens in the Files
//!    panel in the copy's place while the page can read it
//!    (`docs/decisions/0003-attachment-opens-its-file.md`).
//!
//! The command returns no medium: the attachment is for the user.
//!
//! The same copy is made for a picture of the Host that a reply shows, when
//! the reply ends ([`copy_picture`],
//! `docs/decisions/0046-a-reply-keeps-the-pictures-it-shows.md`): the Host is
//! then looked at, never woken, only while it is the device the agent ran on,
//! and the file must be an image.
//!
//! `download` writes attachments of the conversation, the user's or the
//! agent's, into the invocation's directory. The runner downloads each from
//! this backend as every file Demi downloads, resumed after a break
//! (`crates/shared-artifacts/src/download.rs`), and takes the first free name,
//! the same bytes under the name being that file.
//!
//! An attachment is a copy of the file's bytes as they were when the command
//! ran, and nothing changes or removes it afterwards, so a message shows it
//! whatever becomes of the file, the Host or the conversation. Its number and
//! its record are stored in one transaction, and a Fork's destination keeps its
//! source's attachments, since its history names them.
//!
//! A send writes nothing to a Host: the model reads an image, a video or a PDF
//! from the message itself, and fetches a file with `download` only for a
//! command that works on it, which never replaces a file.

use std::rc::{Rc, Weak};

use bytes::Bytes;
use demi_agent_store::{attachments::upload_media_type, pixels::pixel_size};
use demi_backend_blobs::blobs::OPENING_BYTES;
use demi_backend_database::conversation_attachments::{
    self, AttachmentNumber, AttachmentRow, AttachmentSource, StoredAttachment,
};
use demi_backend_database::conversations::ConversationDb;
use demi_backend_database::{StorageError, sequences};
use demi_backend_remote_host::collect_pipe;
use demi_runner_protocol::wire::FileHash;
use demi_host_interface::{
    ByteRange, Call, GroupBuilder, Host as _, HostError, HostErrorKind, LeafBuilder, RpcError, RpcPort, TypedRpc, WhenExists, WriteOptions,
};
use demi_shared_types::{
    BlobRef, NodeId, PictureFailure, PixelSize, Sequence, preview_media_type,
    output::{Cell, Table},
};
use demi_web_api_protocol::attachments::ATTACHMENT_MAX_BYTES;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::HostShard;
use crate::access::{ConversationHost, HostAccessError};
use crate::host_commands::{conversation_of, verb};

const SUMMARY: &str = "Give the user files of this host as attachments of the conversation, which your messages show, and fetch the conversation's attachments as files.";

/// The group's entry in the model's capability index.
const ENTRY: &str = "Makes a Host file an attachment of the conversation, or downloads one.";

const UPLOAD_SUMMARY: &str = "Copy files into the conversation as attachments a1, a2, ..., each kept as it is now, whatever later becomes of the file, and print each one's number. Deliver one in your reply as <attachment id=\"a3\" /> on a line of its own, not also a link to its path: an image shows, a video and audio play, and any other file is a card the user opens. A file is at most 25 MiB. Returns no medium: an attachment is for the user, not for you to see.";

const DOWNLOAD_SUMMARY: &str = "Write attachments of the conversation, such as a file the user sent as <attachment id=\"a7\" ... />, into the current directory, or the one --dir names, under their names, and print each one's path. A file never replaces another: it takes the first free name, and the same file already there is that file.";

/// What the media type falls back to when neither the bytes nor the file's
/// name tell it.
const UNKNOWN_MEDIA_TYPE: &str = "application/octet-stream";

/// The input of `demi attachment upload`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UploadArgs {
    /// Files to upload, relative to the current directory or absolute
    #[schemars(length(min = 1))]
    path: Vec<String>,
}

/// An attachment of `demi attachment upload --json`, which lists them in
/// the order of their paths.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct UploadedAttachment {
    /// Its number, such as `a3`.
    id: String,
    /// The path as it was given.
    path: String,
    media_type: String,
    /// In bytes.
    size: u64,
}

/// The input of `demi attachment download`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DownloadArgs {
    /// Attachments to download, such as a7
    #[schemars(length(min = 1))]
    id: Vec<String>,
    /// Directory to write them into, relative to the current directory or
    /// absolute; the current directory when omitted
    dir: Option<String>,
}

/// An attachment of `demi attachment download --json`, which lists them in
/// the order of their numbers as given.
#[derive(Serialize, JsonSchema)]
struct DownloadedAttachment {
    /// Its number, such as `a7`.
    id: String,
    /// Where it was written, as the directory was given joined with its
    /// name.
    path: String,
}

/// The `attachment` group, whose handlers act in `shard`.
pub fn attachment_group(shard: Weak<dyn HostShard>) -> GroupBuilder {
    GroupBuilder::new("attachment", SUMMARY)
        .index_entry(ENTRY)
        .leaf(
            LeafBuilder::rpc("upload", UPLOAD_SUMMARY)
                .input::<UploadArgs>()
                .positionals(["path"])
                .json_output::<Vec<UploadedAttachment>>()
                .success_output("a table of the files uploaded: each one's attachment id, such as a3, its media type, size and path")
                .failure_output("a file that cannot be uploaded writes its path and why to stderr; the others are still uploaded, and the command exits 1")
                .bind(TypedRpc::new(verb(shard.clone(), upload))),
        )
        .leaf(
            LeafBuilder::rpc("download", DOWNLOAD_SUMMARY)
                .input::<DownloadArgs>()
                .positionals(["id"])
                .json_output::<Vec<DownloadedAttachment>>()
                .success_output("a table of the attachments written: each one's id and path")
                .failure_output("an attachment that cannot be written writes its id and why to stderr; the others are still written, and the command exits 1")
                .bind(TypedRpc::new(verb(shard, download))),
        )
}

async fn download(
    shard: Rc<dyn HostShard>,
    call: Call<DownloadArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let Call { args, invocation } = call;
    let conversation = conversation_of(&invocation)?;
    let device = DeviceId::try_from(invocation.host.as_str())
        .map_err(|_| RpcError::Failed(format!("{}: names no device", invocation.host)))?;
    let directory = args.dir.as_deref().unwrap_or(".");
    let mut written = Vec::new();
    let mut failed = false;
    for id in args.id {
        let fetched = tokio::select! {
            fetched = fetch(&*shard, &conversation, &device, &invocation.cwd, directory, &id) => fetched,
            () = port.cancelled() => return Ok(130),
        };
        match fetched {
            Ok(name) => written.push(DownloadedAttachment {
                path: if directory == "." { name } else { format!("{}/{name}", directory.trim_end_matches(['/', '\\'])) },
                id,
            }),
            Err(reason) => {
                failed = true;
                port.stderr(format!("{}: {id}: {reason}\n", invocation.path.join(" ")))
                    .await?;
            }
        }
    }
    let printed = if invocation.json {
        let json = serde_json::to_string(&written)
            .map_err(|error| RpcError::Failed(error.to_string()))?;
        format!("{json}\n")
    } else {
        let mut table = Table::new(["id", "path"]);
        for attachment in &written {
            table.row::<Cell>([attachment.id.as_str().into(), attachment.path.as_str().into()]);
        }
        table.render()
    };
    port.stdout(printed).await?;
    Ok(u8::from(failed))
}

/// Writes the conversation's attachment `id` into `directory`, beside `cwd`,
/// on the conversation's Host on `device`, and answers the name it took.
/// Answers why when it cannot.
async fn fetch(
    shard: &dyn HostShard,
    conversation: &ConversationId,
    device: &DeviceId,
    cwd: &str,
    directory: &str,
    id: &str,
) -> Result<String, String> {
    let unknown = || format!("the conversation has no attachment {id}");
    let number: AttachmentNumber = id.parse().map_err(|_| unknown())?;
    let row = shard
        .conversation_db(conversation)
        .read(move |connection| conversation_attachments::read(connection, number))
        .await
        .map_err(|error| error.to_string())?
        .flatten()
        .ok_or_else(unknown)?;
    let bytes = shard
        .blobs()
        .get(&row.blob)
        .await
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("the bytes of {id} are no longer stored"))?;
    let options = WriteOptions {
        exists: WhenExists::Rename,
    };
    // The admission waits only as long as the call: a stopped call drops it.
    shard
        .with_host(conversation, Some(device), &CancellationToken::new(), async |host| {
            let paths = host.host.identity().paths;
            let path = paths.join(&paths.resolve(cwd, directory), &row.name);
            host.host
                .download_file(&path, bytes, row.blob.as_str().to_owned(), options)
                .await
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.message)
}

async fn upload(
    shard: Rc<dyn HostShard>,
    call: Call<UploadArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let Call { args, invocation } = call;
    let conversation = conversation_of(&invocation)?;
    let device = DeviceId::try_from(invocation.host.as_str())
        .map_err(|_| RpcError::Failed(format!("{}: names no device", invocation.host)))?;
    // The node of the agent that runs the command, a side question's
    // included, is recorded as the files' uploader; none when the user runs it.
    let node = invocation.caller.as_ref().map(|caller| caller.node.clone());
    let mut uploaded = Vec::new();
    let mut failed = false;
    for path in args.path {
        let stored = tokio::select! {
            stored = store(&*shard, &conversation, &device, &invocation.cwd, node.clone(), &path, Purpose::Upload) => stored,
            () = port.cancelled() => return Ok(130),
        };
        match stored {
            Ok(row) => uploaded.push(UploadedAttachment {
                id: row.number.to_string(),
                path,
                media_type: row.media_type,
                size: row.size,
            }),
            Err(reason) => {
                failed = true;
                port.stderr(format!("{}: {path}: {reason}\n", invocation.path.join(" ")))
                    .await?;
            }
        }
    }
    let printed = if invocation.json {
        let json = serde_json::to_string(&uploaded)
            .map_err(|error| RpcError::Failed(error.to_string()))?;
        format!("{json}\n")
    } else {
        let mut table = Table::new(["id", "media type", "size (bytes)", "path"]);
        for attachment in &uploaded {
            table.row::<Cell>([
                attachment.id.as_str().into(),
                attachment.media_type.as_str().into(),
                attachment.size.into(),
                attachment.path.as_str().into(),
            ]);
        }
        table.render()
    };
    port.stdout(printed).await?;
    Ok(u8::from(failed))
}

/// Why a file of the Host is copied into the conversation, which decides how
/// the Host is reached and what the copy may be.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    /// An agent's `demi attachment upload`: the command wakes a stopped Cloud
    /// as any command does, and any file is a copy.
    Upload,
    /// A picture a reply shows: the Host is looked at, never woken, and only
    /// while it is the device the agent ran on; the file must be an image.
    Picture,
}

/// Why a file of the Host was not copied into the conversation. The words
/// are the upload command's; a picture of a reply keeps the cause alone
/// ([`CopyError::failure`]).
#[derive(Debug, thiserror::Error)]
enum CopyError {
    /// The Host has no such file.
    #[error("{0}")]
    Missing(String),
    /// The file is over the limit of an attachment.
    #[error("the file is {size} bytes; an attachment is at most 25 MiB ({} bytes)", ATTACHMENT_MAX_BYTES)]
    TooLarge { size: u64 },
    /// The file is no image, and a picture must be.
    #[error("the file is not an image ({0})")]
    NotAnImage(String),
    /// The Host could not be reached for the copy.
    #[error("{0}")]
    HostUnavailable(String),
    /// The Host could not read the file, or the copy could not be kept.
    #[error("{0}")]
    Unreadable(String),
}

impl CopyError {
    /// The cause a reply keeps with the picture.
    fn failure(&self) -> PictureFailure {
        match self {
            Self::Missing(_) => PictureFailure::Missing,
            Self::TooLarge { size } => PictureFailure::TooLarge { size: *size },
            Self::NotAnImage(media_type) => PictureFailure::NotAnImage {
                media_type: media_type.clone(),
            },
            Self::HostUnavailable(_) => PictureFailure::HostUnavailable,
            Self::Unreadable(_) => PictureFailure::Unreadable,
        }
    }
}

/// A failure the Host answered: no such file, or no way to reach it, or
/// neither.
impl From<HostError> for CopyError {
    fn from(error: HostError) -> Self {
        match &error.kind {
            HostErrorKind::Failed { code } if matches!(code.as_deref(), Some("ENOENT" | "ENOTDIR")) => {
                Self::Missing(error.message)
            }
            HostErrorKind::Offline | HostErrorKind::Unavailable | HostErrorKind::Interrupted => {
                Self::HostUnavailable(error.message)
            }
            HostErrorKind::Failed { .. } | HostErrorKind::TooLarge | HostErrorKind::Protocol => {
                Self::Unreadable(error.message)
            }
        }
    }
}

/// Why the conversation admitted no operation on its Host.
impl From<HostAccessError> for CopyError {
    fn from(error: HostAccessError) -> Self {
        match error {
            HostAccessError::Host(error) => error.into(),
            HostAccessError::Refused(_)
            | HostAccessError::Cloud(_)
            | HostAccessError::Cancelled
            | HostAccessError::Missing => Self::HostUnavailable(error.to_string()),
            HostAccessError::Storage(_) | HostAccessError::Objects(_) | HostAccessError::Store(_) => {
                Self::Unreadable(error.to_string())
            }
        }
    }
}

/// Runs `operation` on the conversation's Host `device` as `purpose` reaches
/// it, or answers why it cannot. Each admission waits only as long as the
/// call: a stopped call drops it.
async fn on_host<T>(
    shard: &dyn HostShard,
    conversation: &ConversationId,
    device: &DeviceId,
    purpose: Purpose,
    operation: impl AsyncFnOnce(&ConversationHost) -> T,
) -> Result<T, CopyError> {
    let cancel = CancellationToken::new();
    match purpose {
        Purpose::Upload => Ok(shard.with_host(conversation, Some(device), &cancel, operation).await?),
        Purpose::Picture => shard
            .look_at_host_on(conversation, device, &cancel, operation)
            .await?
            .ok_or_else(|| {
                CopyError::HostUnavailable("the conversation no longer works on the Host the reply was written on".to_owned())
            }),
    }
}

/// Copies the picture at `path`, beside `cwd`, from the conversation's Host
/// on `device` into its attachments, as the reply of the agent of `node`
/// shows it (`docs/decisions/0046-a-reply-keeps-the-pictures-it-shows.md`),
/// and answers its number. The Host is looked at, never woken, and only while
/// it is `device`; a file that is no image, or is over the limit of an
/// attachment, is refused. Answers why when it cannot.
pub async fn copy_picture(
    shard: &dyn HostShard,
    conversation: &ConversationId,
    device: &DeviceId,
    cwd: &str,
    node: NodeId,
    path: &str,
) -> Result<String, PictureFailure> {
    let row = store(shard, conversation, device, cwd, Some(node), path, Purpose::Picture)
        .await
        .map_err(|error| {
            tracing::debug!(%path, %error, "a picture of a reply was not copied");
            error.failure()
        })?;
    Ok(row.number.to_string())
}

/// Copies the file at `path`, beside `cwd`, from the conversation's Host on
/// `device`, and stores it as the conversation's next attachment, the upload of
/// the agent of `node`: its blob first, then its record.
///
/// The Host hashes the file first, and sends its bytes only when the namespace
/// does not hold them already. Answers why when it cannot.
async fn store(
    shard: &dyn HostShard,
    conversation: &ConversationId,
    device: &DeviceId,
    cwd: &str,
    node: Option<NodeId>,
    path: &str,
    purpose: Purpose,
) -> Result<AttachmentRow, CopyError> {
    let hash = on_host(shard, conversation, device, purpose, async |host| {
        host.host
            .hash_file_in(cwd, path, ATTACHMENT_MAX_BYTES as u64)
            .await
    })
    .await??;
    // The full path of the file the Host read, as it writes it, which a file
    // card opens in the Files panel in the copy's place.
    let (sha256, full_path) = match hash {
        // A file that grew past the limit since the Host looked at its size
        // is refused all the same.
        FileHash::Hashed { size, sha256, path } => {
            within_limit(size)?;
            (sha256, path)
        }
        FileHash::TooLarge { size } => return Err(CopyError::TooLarge { size }),
    };
    let blobs = shard.blobs();
    let held = blobs
        .opening(&sha256)
        .await
        .map_err(|error| CopyError::Unreadable(error.to_string()))?;
    // The media type and an image's or a video's size come from the file's
    // opening, so a held blob is read only that far, and a file read whole
    // is read only that far too: the same file gets the same record.
    let (blob, opening, size) = match held {
        Some(opening) => (sha256, opening.bytes, opening.size),
        None => {
            let bytes = on_host(shard, conversation, device, purpose, async |host| {
                read(&host.host, cwd, path).await
            })
            .await??;
            let size = u64::try_from(bytes.len()).expect("an attachment's size fits u64");
            let blob = blobs
                .put(bytes.clone())
                .await
                .map_err(|error| CopyError::Unreadable(error.to_string()))?;
            let opened = bytes.len().min(OPENING_BYTES as usize);
            (blob, bytes.slice(..opened), size)
        }
    };
    let sent = preview_media_type(path).unwrap_or(UNKNOWN_MEDIA_TYPE);
    let media_type = upload_media_type(sent, &opening);
    if purpose == Purpose::Picture && !media_type.starts_with("image/") {
        return Err(CopyError::NotAnImage(media_type));
    }
    let pixels = pixel_size(opening, &media_type);
    let name = path
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_owned();
    let attachment = Unnumbered {
        name,
        media_type,
        size,
        pixels,
        blob,
        node,
        source: Some(AttachmentSource {
            host: device.clone(),
            path: full_path,
        }),
    };
    let announce = {
        let shard = shard.this();
        let conversation = conversation.clone();
        move || shard.summary_changed(&conversation)
    };
    commit(shard.tasks(), shard.conversation_db(conversation), attachment, announce)
        .await
        .map_err(CopyError::Unreadable)
}

/// An attachment's record before the conversation gives it its number.
pub(crate) struct Unnumbered {
    pub(crate) name: String,
    pub(crate) media_type: String,
    pub(crate) size: u64,
    pub(crate) pixels: Option<PixelSize>,
    pub(crate) blob: BlobRef,
    /// The node of the agent that uploads it; none for a file the user
    /// sends (`conversation_attachments::StoredAttachment`).
    pub(crate) node: Option<NodeId>,
    /// The file it is copied from; none for a file the user sends.
    pub(crate) source: Option<AttachmentSource>,
}

/// Stores `attachment` as the conversation's next one in `db`, its number and
/// its record in one transaction, so a summary that names the number names a
/// record the page can read (`conversation_attachments::last`). Then calls
/// `announce`, which sends the summary to the user's pages.
///
/// The write and the announcement run in a task of `tasks`, which the shard's
/// close waits for. A call stopped once the write is on its way, whose future
/// is dropped, cuts neither, since the database's thread commits a write it was
/// handed whoever still waits for it. So a stored attachment is always
/// announced.
pub(crate) fn commit(
    tasks: &TaskTracker,
    db: ConversationDb,
    attachment: Unnumbered,
    announce: impl FnOnce() + 'static,
) -> impl Future<Output = Result<AttachmentRow, String>> {
    let committed = tasks.spawn_local(async move {
        let commit = db.commit_point();
        let row = db
            .call(move |connection| {
                let transaction = connection.transaction()?;
                let row = AttachmentRow {
                    number: AttachmentNumber(sequences::next(&transaction, Sequence::Attachment)?),
                    name: attachment.name,
                    media_type: attachment.media_type,
                    size: attachment.size,
                    pixels: attachment.pixels,
                    blob: attachment.blob,
                    source: attachment.source,
                };
                let stored = StoredAttachment {
                    row,
                    node: attachment.node,
                };
                conversation_attachments::insert(&transaction, std::slice::from_ref(&stored))?;
                commit.commit(transaction)?;
                Ok(stored.row)
            })
            .await?;
        announce();
        Ok::<_, StorageError>(row)
    });
    async move {
        committed
            .await
            .map_err(|error| error.to_string())?
            .map_err(|error| error.to_string())
    }
}

/// Refuses a file over the limit of the user's own uploads.
fn within_limit(size: u64) -> Result<(), CopyError> {
    if size > ATTACHMENT_MAX_BYTES as u64 {
        return Err(CopyError::TooLarge { size });
    }
    Ok(())
}

/// The bytes of the file at `path` beside `cwd`. A file that grew past the
/// limit since it was hashed is refused before a byte is read.
async fn read(
    host: &demi_backend_remote_host::RemoteHost,
    cwd: &str,
    path: &str,
) -> Result<Bytes, CopyError> {
    let opened = host.read_pipe_in(cwd, path, ByteRange::default()).await?;
    // Dropping the reader stops the read.
    within_limit(opened.stat.size)?;
    Ok(collect_pipe(opened.body, ATTACHMENT_MAX_BYTES).await?)
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use demi_backend_database::conversations::{ConversationBlobs, ConversationStores, MAX_WRITERS};
    use futures_util::future::join_all;

    use super::*;

    const CONVERSATION: &str = "0193f1e2-7c4d-7e8f-9a0b-1c2d3e4f5a6b";

    /// Blobs no migration reads: the databases are new.
    struct NoBlobs;

    impl ConversationBlobs for NoBlobs {
        fn read(&self, _: &ConversationId, _: &BlobRef) -> Result<Option<Bytes>, String> {
            Ok(None)
        }
    }

    async fn stores(directory: &Path) -> ConversationStores {
        ConversationStores::open(directory.to_owned(), MAX_WRITERS, Arc::new(NoBlobs))
            .await
            .unwrap()
    }

    fn shot(name: &str) -> Unnumbered {
        Unnumbered {
            name: name.to_owned(),
            media_type: "image/png".to_owned(),
            size: 4,
            pixels: Some(PixelSize { width: 2, height: 1 }),
            blob: BlobRef::of(name.as_bytes()),
            node: None,
            source: None,
        }
    }

    /// The conversation's database, the one file the directory holds.
    fn database(directory: &Path) -> PathBuf {
        std::fs::read_dir(directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.extension().is_some_and(|extension| extension == "sqlite"))
            .expect("the first write created the database")
    }

    /// The numbers of the attachments committed now, read beside the writer
    /// as a page's read of the attachment route is.
    fn committed(directory: &Path) -> Vec<u64> {
        let connection = rusqlite::Connection::open_with_flags(
            database(directory),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        conversation_attachments::all(&connection)
            .unwrap()
            .into_iter()
            .map(|stored| stored.row.number.0)
            .collect()
    }

    /// A call stopped once its attachment's write is on the database's thread
    /// drops its future. The write commits all the same, and the pages still
    /// hear of it, or a reply that named the number before the upload would
    /// show it missing until something else sent the summary. The commit is
    /// held until the call has stopped.
    ///
    /// Planted defect it catches: the write and the announcement awaited in the
    /// call's own future, which the stop drops after the write is handed over
    /// and before the announcement.
    ///
    /// Cost: instant.
    #[tokio::test(flavor = "local")]
    async fn an_attachment_whose_call_stops_once_its_write_is_sent_is_stored_and_announced() {
        let directory = tempfile::tempdir().unwrap();
        let stores = stores(directory.path()).await;
        let db = stores.db(&ConversationId::try_from(CONVERSATION).unwrap());
        let tasks = TaskTracker::new();
        let (announce, announced) = tokio::sync::oneshot::channel();
        let hold = stores.hold_commits();

        let committing = commit(&tasks, db, shot("login.png"), move || {
            announce.send(()).unwrap();
        });
        tokio::select! {
            _ = committing => panic!("the commit waits at the hold"),
            () = hold.until_waiting(1) => {}
        }
        // The call has stopped, its future dropped; the write waits at its
        // commit on the database's thread.
        hold.release();

        announced.await.expect("the stored attachment is announced");
        assert_eq!(committed(directory.path()), [1]);
        tasks.close();
        tasks.wait().await;
    }

    /// Uploads at once, as several `demi attachment upload` calls or the files
    /// of one, take distinct numbers in the order they commit. Each is
    /// announced once its record, and every one below it, can be read: a page
    /// that sees `lastAttachment` 5 finds a1 to a5, and treats a number up to
    /// it without a record as missing for good.
    ///
    /// Planted defects it catches:
    ///
    /// - A number taken apart from the record's transaction, such as the
    ///   highest one plus 1, which two writes can both take.
    /// - An announcement before the commit.
    ///
    /// Cost: instant.
    #[tokio::test(flavor = "local")]
    async fn attachments_stored_at_once_take_distinct_numbers_each_announced_once_readable() {
        const UPLOADS: u64 = 8;
        let directory = tempfile::tempdir().unwrap();
        let stores = stores(directory.path()).await;
        let db = stores.db(&ConversationId::try_from(CONVERSATION).unwrap());
        let tasks = TaskTracker::new();
        let seen = Rc::new(RefCell::new(Vec::new()));

        let rows = join_all((1..=UPLOADS).map(|index| {
            let seen = seen.clone();
            let path = directory.path().to_owned();
            commit(&tasks, db.clone(), shot(&format!("shot-{index}.png")), move || {
                seen.borrow_mut().push(committed(&path));
            })
        }))
        .await;

        let mut numbers: Vec<u64> = rows.into_iter().map(|row| row.unwrap().number.0).collect();
        numbers.sort_unstable();
        assert_eq!(numbers, (1..=UPLOADS).collect::<Vec<_>>());
        let seen = seen.borrow();
        assert_eq!(seen.len(), UPLOADS as usize, "each attachment is announced once");
        for (announced, present) in (1..).zip(seen.iter()) {
            // Later commits may land before an announcement, never a gap.
            let latest = present.last().copied().unwrap_or(0);
            assert!(latest >= announced, "announcement {announced} saw {present:?}");
            assert_eq!(*present, (1..=latest).collect::<Vec<_>>(), "announcement {announced}");
        }
        tasks.close();
        tasks.wait().await;
    }
}
