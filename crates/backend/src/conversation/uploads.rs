//! The uploads a frame's content names (`backend.md` § Media by reference):
//! an upload of the conversation's owner is written to the conversation's
//! Host under `~/.demi/attachments/<conversation>/`, outside every working
//! directory, and becomes its native media block, when it has one, then its
//! attachment record. An upload that is gone, another user's, or whose bytes
//! are missing becomes the text that says it is not available.

use std::path::Path;

use bytes::Bytes;
use demi_agent::attachments::{Upload, unavailable, upload_blocks};
use demi_core::UserContentBlock;
use demi_host_remote::RemoteHost;
use demi_shell::{FileContents, Host as _, HostError, WriteOptions};
use demi_web_api::ids::{AttachmentId, ConversationId};

use super::host_access::{ConversationHost, HostAccessError};
use crate::shard::Shard;

/// Where a Host keeps the conversations' attachments, under its account's
/// home.
const ATTACHMENTS_DIR: &str = ".demi/attachments";

impl Shard {
    /// The blocks the upload `reference` becomes in a message of the user's
    /// conversation `id`, written under `file_name` on `host`, the Host the
    /// frame was admitted on.
    pub(crate) async fn resolve_upload(
        &self,
        id: &ConversationId,
        host: &ConversationHost,
        reference: &str,
        file_name: &str,
    ) -> Result<Vec<UserContentBlock>, HostAccessError> {
        let not_available = || Ok(vec![unavailable(reference)]);
        let Ok(attachment) = AttachmentId::try_from(reference) else {
            return not_available();
        };
        let services = self.services();
        let record = services.control.attachment(attachment).await?;
        let Some(record) = record.filter(|record| record.owner == *self.user()) else {
            return not_available();
        };
        let Some(bytes) = services.blobs.for_user(self.user()).get(&record.sha256).await? else {
            return not_available();
        };
        let written = write_attachment(&host.host, id, file_name, bytes.clone()).await?;
        Ok(upload_blocks(Upload {
            name: &written.name,
            path: &written.path,
            media_type: &record.media_type,
            sha256: &record.sha256,
            bytes: &bytes,
        }))
    }
}

/// Where an attachment was written.
struct Written {
    /// The name it has there: the one it was sent with, or that name with a
    /// number when a file had it already.
    name: String,
    /// Absolute on the Host.
    path: String,
}

/// Writes `bytes` into the conversation's attachment directory on `host`,
/// under `file_name` or, when a file has that name already, the first free
/// `name-2.ext`, `name-3.ext` and so on; nothing is overwritten, and what is
/// written stays, so the path a transcript names stays readable.
async fn write_attachment(
    host: &RemoteHost,
    id: &ConversationId,
    file_name: &str,
    bytes: Bytes,
) -> Result<Written, HostError> {
    let home = host.identity().home_dir;
    let directory = format!("{}/{ATTACHMENTS_DIR}/{id}", home.trim_end_matches('/'));
    let mut name = file_name.to_owned();
    let mut number = 2;
    while host.fs().exists(&format!("{directory}/{name}")).await? {
        name = numbered(file_name, number);
        number += 1;
    }
    let path = format!("{directory}/{name}");
    let options = WriteOptions { create_parents: true };
    host.fs().write_file(&path, FileContents::Bytes(bytes), options).await?;
    Ok(Written { name, path })
}

/// `file_name` with `number` before its extension: `notes-2.txt`, and
/// `.env-2` for a name that is all extension.
fn numbered(file_name: &str, number: u32) -> String {
    let name = Path::new(file_name);
    match (name.file_stem(), name.extension()) {
        (Some(stem), Some(extension)) => {
            format!("{}-{number}.{}", stem.to_string_lossy(), extension.to_string_lossy())
        }
        _ => format!("{file_name}-{number}"),
    }
}
