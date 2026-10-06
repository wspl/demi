//! The product's `demi attachment` group (`commands.md` § Attachment
//! commands): `upload` copies files of the Host into the conversation as
//! attachments `a1`, `a2`, …, which the agent's messages then show as
//! `attachment:a3`. The runner of the conversation's Host reads each file
//! beside the invocation and streams its bytes to the backend through a pipe
//! (`runner.md` § File contents); the backend reads the media type from the
//! bytes as it does for the user's uploads, stores the blob in the
//! conversation owner's namespace, and then the record, under the next
//! number of the conversation's `attachment` sequence. The command returns
//! no medium: the attachment is for the user.

use std::rc::{Rc, Weak};

use bytes::Bytes;
use demi_agent_store::attachments::upload_media_type;
use demi_backend_database::conversation_attachments::{self, AttachmentNumber, AttachmentRow};
use demi_backend_database::sequences;
use demi_backend_remote_host::collect_pipe;
use demi_host_interface::text::{byte_size, table};
use demi_host_interface::{ByteRange, Call, GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_shared_types::{Sequence, preview_media_type};
use demi_web_api_protocol::attachments::ATTACHMENT_MAX_BYTES;
use demi_web_api_protocol::ids::ConversationId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::HostShard;
use crate::host_commands::{conversation_of, verb};

const SUMMARY: &str = "Give the user files of this host as attachments of the conversation, which your messages show.";

const UPLOAD_SUMMARY: &str = "Copy files into the conversation as attachments a1, a2, ..., each kept as it is now, whatever later becomes of the file, and print each one's number. Show one in a message as ![description](attachment:a3): an image shows and a video plays; link any file as [name](attachment:a3). A file is at most 25 MiB. Returns no medium: an attachment is for the user, not for you to see.";

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
            .success_output("one line per file: its number, path, media type and size, such as `a3  out/login.png  image/png  402 KB`")
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
    let mut uploaded = Vec::new();
    let mut failed = false;
    for path in args.path {
        let stored = tokio::select! {
            stored = store(&*shard, &conversation, &invocation.cwd, &path) => stored,
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
                    byte_size(attachment.size),
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

/// Reads the file at `path`, beside `cwd`, from the conversation's Host,
/// and stores it as the conversation's next attachment: its blob first, then
/// its record. Answers why when it cannot.
async fn store(
    shard: &dyn HostShard,
    conversation: &ConversationId,
    cwd: &str,
    path: &str,
) -> Result<AttachmentRow, String> {
    // The admission waits only as long as the call: a stopped call drops it.
    let bytes = shard
        .with_host(conversation, None, &CancellationToken::new(), async |host| {
            read(&host.host, cwd, path).await
        })
        .await
        .map_err(|error| error.to_string())??;
    let sent = preview_media_type(path).unwrap_or(UNKNOWN_MEDIA_TYPE);
    let media_type = upload_media_type(sent, &bytes);
    let size = u64::try_from(bytes.len()).expect("an attachment's size fits u64");
    let blob = shard
        .blobs()
        .put(bytes)
        .await
        .map_err(|error| error.to_string())?;
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
        blob,
    };
    let record = row.clone();
    db.call(move |connection| conversation_attachments::insert(connection, &[record]))
        .await
        .map_err(|error| error.to_string())?;
    Ok(row)
}

/// The bytes of the file at `path` beside `cwd`: a file over the limit of the
/// user's own uploads is refused before a byte is read.
async fn read(
    host: &demi_backend_remote_host::RemoteHost,
    cwd: &str,
    path: &str,
) -> Result<Bytes, String> {
    let opened = host
        .read_pipe_in(cwd, path, ByteRange::default())
        .await
        .map_err(|error| error.message)?;
    let limit = ATTACHMENT_MAX_BYTES as u64;
    if opened.stat.size > limit {
        // Dropping the reader stops the read.
        return Err(format!(
            "the file is {} bytes; an attachment is at most 25 MiB ({limit} bytes)",
            opened.stat.size
        ));
    }
    collect_pipe(opened.body, ATTACHMENT_MAX_BYTES)
        .await
        .map_err(|error| error.message)
}
