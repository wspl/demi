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
use demi_backend_database::sequences;
use demi_backend_remote_host::collect_pipe;
use demi_runner_protocol::wire::FileHash;
use demi_host_interface::text::table;
use demi_host_interface::{ByteRange, Call, GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_shared_types::{Sequence, preview_media_type};
use demi_web_api_protocol::attachments::ATTACHMENT_MAX_BYTES;
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::HostShard;
use crate::host_commands::{conversation_of, verb};

const SUMMARY: &str = "Give the user files of this host as attachments of the conversation, which your messages show.";

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

/// `demi attachment upload --json`: the attachments stored, in the order of
/// their paths.
#[derive(Serialize, JsonSchema)]
struct Uploaded {
    attachments: Vec<UploadedAttachment>,
}

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
    GroupBuilder::new("attachment", SUMMARY).leaf(
        LeafBuilder::rpc("upload", UPLOAD_SUMMARY)
            .input::<UploadArgs>()
            .positionals(["path"])
            .json_output::<Uploaded>()
            .success_output("one line per file: its number, path, media type and size, such as `a3  out/login.png  image/png  421888 bytes`")
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
        .map_err(|_| RpcError::Failed(format!("{} names no device", invocation.host)))?;
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
                port.stderr(format!("demi attachment upload: {path}: {reason}\n"))
                    .await?;
            }
        }
    }
    let printed = if invocation.json {
        let answer = Uploaded {
            attachments: uploaded,
        };
        let json = serde_json::to_string(&answer)
            .map_err(|error| RpcError::Failed(error.to_string()))?;
        format!("{json}\n")
    } else {
        let rows: Vec<[String; 4]> = uploaded
            .into_iter()
            .map(|attachment| {
                [
                    attachment.id,
                    attachment.path,
                    attachment.media_type,
                    format!("{} bytes", attachment.size),
                ]
            })
            .collect();
        table(&rows)
    };
    if !printed.is_empty() {
        port.stdout(printed).await?;
    }
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
    let db = shard.conversation_db(conversation);
    let number = db
        .call(|connection| sequences::next(connection, Sequence::Attachment))
        .await
        .map_err(|error| error.to_string())?;
    let name = path
        .rsplit(['/', '\\'])
        .next()
        .filter(|name| !name.is_empty())
        .unwrap_or(path)
        .to_owned();
    let row = AttachmentRow {
        number: AttachmentNumber(number),
        name,
        media_type,
        size,
        pixels,
        blob,
    };
    let record = row.clone();
    db.call(move |connection| conversation_attachments::insert(connection, &[record]))
        .await
        .map_err(|error| error.to_string())?;
    Ok(row)
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
