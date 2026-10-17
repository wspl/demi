//! The product's `demi attachment` group (`commands.md` § Attachment
//! commands): `upload` copies files of the Host into the conversation as
//! attachments `a1`, `a2`, …, which the agent's messages then show as
//! `attachment:a3`. The runner of the invocation's Host, the conversation's
//! primary Host or an attached one a `demi host shell` job runs on, first
//! hashes each file beside the invocation, through the conversation's host
//! access, and refuses one over 25 MiB before reading a byte; a blob the
//! conversation owner's namespace holds already is not read again, only its
//! first 64 KiB from the object store for its media type. Otherwise the runner streams the file's bytes to the backend
//! through a pipe (`runner.md` § File contents), and the backend stores the
//! blob in that namespace. The backend reads the media type from the bytes
//! as it does for the user's uploads, and stores the record under the next
//! number of the conversation's `attachment` sequence. The command returns
//! no medium: the attachment is for the user.

use std::rc::{Rc, Weak};

use bytes::Bytes;
use demi_agent_store::{attachments::upload_media_type, pixels::pixel_size};
use demi_backend_blobs::blobs::OPENING_BYTES;
use demi_backend_database::conversation_attachments::{self, AttachmentNumber, AttachmentRow};
use demi_backend_database::conversations::ConversationDb;
use demi_backend_database::{StorageError, sequences};
use demi_backend_remote_host::collect_pipe;
use demi_runner_protocol::wire::FileHash;
use demi_host_interface::{ByteRange, Call, GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_shared_types::{
    BlobRef, PixelSize, Sequence, preview_media_type,
    output::{Cell, Table},
};
use demi_web_api_protocol::attachments::ATTACHMENT_MAX_BYTES;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;

use crate::HostShard;
use crate::host_commands::{conversation_of, verb};

const SUMMARY: &str = "Give the user files of this host as attachments of the conversation, which your messages show.";

/// The group's entry in the model's capability index (`system-prompt.md`
/// § Capability index).
const ENTRY: &str = "Gives the user files from the Host as attachments of the conversation, which stay viewable whatever later becomes of the file. Use it to deliver a screenshot, a recording, a download or a file you made, and embed it in your reply as `![…](attachment:a3)`. A file the user should see as it changes on the Host is linked by its path instead.";

const UPLOAD_SUMMARY: &str = "Copy files into the conversation as attachments a1, a2, ..., each kept as it is now, whatever later becomes of the file, and print each one's number. Show one in a message as ![description](attachment:a3): an image shows and a video plays; link any other file as [name](attachment:a3). A file is at most 25 MiB. Returns no medium: an attachment is for the user, not for you to see.";

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
                .bind(TypedRpc::new(verb(shard, upload))),
        )
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
    let mut uploaded = Vec::new();
    let mut failed = false;
    for path in args.path {
        let stored = tokio::select! {
            stored = store(&*shard, &conversation, &device, &invocation.cwd, &path) => stored,
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

/// Copies the file at `path`, beside `cwd`, from the conversation's Host on
/// `device`, and stores it as the conversation's next attachment: its blob
/// first, then its record. The Host hashes the file first, and sends its
/// bytes only when the namespace does not hold them already. Answers why
/// when it cannot.
async fn store(
    shard: &dyn HostShard,
    conversation: &ConversationId,
    device: &DeviceId,
    cwd: &str,
    path: &str,
) -> Result<AttachmentRow, String> {
    // Each admission waits only as long as the call: a stopped call drops it.
    let hash = shard
        .with_host(conversation, Some(device), &CancellationToken::new(), async |host| {
            host.host
                .hash_file_in(cwd, path, ATTACHMENT_MAX_BYTES as u64)
                .await
        })
        .await
        .map_err(|error| error.to_string())?
        .map_err(|error| error.message)?;
    let sha256 = match hash {
        // A file that grew past the limit since the Host looked at its size
        // is refused all the same.
        FileHash::Hashed { size, sha256 } => {
            within_limit(size)?;
            sha256
        }
        FileHash::TooLarge { size } => return Err(over_limit(size)),
    };
    let blobs = shard.blobs();
    let held = blobs
        .opening(&sha256)
        .await
        .map_err(|error| error.to_string())?;
    // The media type and an image's or a video's size come from the file's
    // opening, so a held blob is read only that far, and a file read whole
    // is read only that far too: the same file gets the same record.
    let (blob, opening, size) = match held {
        Some(opening) => (sha256, opening.bytes, opening.size),
        None => {
            let bytes = shard
                .with_host(conversation, Some(device), &CancellationToken::new(), async |host| {
                    read(&host.host, cwd, path).await
                })
                .await
                .map_err(|error| error.to_string())??;
            let size = u64::try_from(bytes.len()).expect("an attachment's size fits u64");
            let blob = blobs
                .put(bytes.clone())
                .await
                .map_err(|error| error.to_string())?;
            let opened = bytes.len().min(OPENING_BYTES as usize);
            (blob, bytes.slice(..opened), size)
        }
    };
    let sent = preview_media_type(path).unwrap_or(UNKNOWN_MEDIA_TYPE);
    let media_type = upload_media_type(sent, &opening);
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
    };
    let announce = {
        let shard = shard.this();
        let conversation = conversation.clone();
        move || shard.summary_changed(&conversation)
    };
    commit(shard.tasks(), shard.conversation_db(conversation), attachment, announce).await
}

/// An attachment's record before the conversation gives it its number.
struct Unnumbered {
    name: String,
    media_type: String,
    size: u64,
    pixels: Option<PixelSize>,
    blob: BlobRef,
}

/// Stores `attachment` as the conversation's next one in `db`, its number
/// and its record in one transaction, so a summary that names the number
/// names a record the page can read (`conversation_attachments::last`), and
/// then calls `announce`, which sends the summary to the user's pages. The
/// write and the announcement run in a task of `tasks`, which the shard's
/// close waits for: a call stopped once the write is on its way, whose
/// future is dropped, cuts neither, since the database's thread commits a
/// write it was handed whoever still waits for it. So a stored attachment
/// is always announced.
fn commit(
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
                };
                conversation_attachments::insert(&transaction, std::slice::from_ref(&row))?;
                commit.commit(transaction)?;
                Ok(row)
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
fn within_limit(size: u64) -> Result<(), String> {
    if size > ATTACHMENT_MAX_BYTES as u64 {
        return Err(over_limit(size));
    }
    Ok(())
}

/// Why a file of `size` bytes, over the limit, is not uploaded.
fn over_limit(size: u64) -> String {
    let limit = ATTACHMENT_MAX_BYTES;
    format!("the file is {size} bytes; an attachment is at most 25 MiB ({limit} bytes)")
}

/// The bytes of the file at `path` beside `cwd`. A file that grew past the
/// limit since it was hashed is refused before a byte is read.
async fn read(
    host: &demi_backend_remote_host::RemoteHost,
    cwd: &str,
    path: &str,
) -> Result<Bytes, String> {
    let opened = host
        .read_pipe_in(cwd, path, ByteRange::default())
        .await
        .map_err(|error| error.message)?;
    // Dropping the reader stops the read.
    within_limit(opened.stat.size)?;
    collect_pipe(opened.body, ATTACHMENT_MAX_BYTES)
        .await
        .map_err(|error| error.message)
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
            .map(|row| row.number.0)
            .collect()
    }

    /// A call stopped once its attachment's write is on the database's
    /// thread drops its future; the write commits all the same, and the
    /// pages still hear of it, or a reply that named the number before the
    /// upload would show it missing until something else sent the summary.
    /// The commit is held until the call has stopped. Instant.
    ///
    /// Planted defect it catches: the write and the announcement awaited
    /// in the call's own future, which the stop drops after the write is
    /// handed over and before the announcement.
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

    /// Uploads at once, as several `demi attachment upload` calls or the
    /// files of one, take distinct numbers in the order they commit, and
    /// each is announced once its record, and every one below it, can be
    /// read: a page that sees `lastAttachment` 5 finds a1 to a5, and treats a
    /// number up to it without a record as missing for good. Instant.
    ///
    /// Planted defects it catches: a number taken apart from the record's
    /// transaction, such as the highest one plus 1, which two writes can
    /// both take; an announcement before the commit.
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
