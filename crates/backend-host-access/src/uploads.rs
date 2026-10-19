//! The uploads a frame's content names (`backend.md` § Media by reference):
//! an upload of the conversation's owner takes the conversation's next
//! attachment number, `a7`, from the sequence `demi attachment upload` takes
//! its numbers from, and becomes its native media block, when it has one,
//! then its attachment record. Nothing is written to a Host: a command that
//! needs the file fetches it with `demi attachment download`. The medium
//! references the upload's own blob, or the blob of the image fitted from
//! it, which is stored here, and its bytes go to the session with it. An
//! upload that is gone, another user's, or whose bytes are missing becomes
//! the text that says it is not available.

use demi_agent_store::attachments::{Upload, unavailable, upload_blocks};
use demi_agent_store::media::HeldMedia;
use demi_agent_store::pixels::pixel_size;
use demi_backend_blobs::blobs::OPENING_BYTES;
use demi_shared_types::{B64Bytes, UserContentBlock};
use demi_web_api_protocol::ids::{AttachmentId, ConversationId};

use crate::HostShard;
use crate::attachment_commands::{Unnumbered, commit};

impl dyn HostShard + '_ {
    /// The blocks the upload `reference` becomes in a message of the user's
    /// conversation `id` under `file_name`, numbered as the conversation's
    /// next attachment, with the bytes of its medium, or why it cannot.
    pub async fn resolve_upload(
        &self,
        id: &ConversationId,
        reference: &str,
        file_name: &str,
    ) -> Result<(Vec<UserContentBlock>, HeldMedia), String> {
        let not_available = || Ok((vec![unavailable(reference)], HeldMedia::default()));
        let Ok(attachment) = AttachmentId::try_from(reference) else {
            return not_available();
        };
        let record = self.control().attachment(attachment).await.map_err(|error| error.to_string())?;
        let Some(record) = record.filter(|record| record.owner == *self.user()) else {
            return not_available();
        };
        let blobs = self.blobs();
        let Some(bytes) = blobs.get(&record.sha256).await.map_err(|error| error.to_string())? else {
            return not_available();
        };
        let opening = bytes.slice(..bytes.len().min(OPENING_BYTES as usize));
        let numbered = Unnumbered {
            name: file_name.to_owned(),
            media_type: record.media_type.clone(),
            size: bytes.len() as u64,
            pixels: pixel_size(opening, &record.media_type),
            blob: record.sha256.clone(),
            // The user's file is no agent's upload, which no reply shows.
            node: None,
        };
        let row = commit(self.tasks(), self.conversation_db(id), numbered, || {}).await?;
        let number = row.number.to_string();
        let upload = Upload {
            name: file_name,
            id: &number,
            media_type: &record.media_type,
            sha256: &record.sha256,
            bytes: &B64Bytes::from(bytes),
        };
        upload_blocks(upload, &blobs).await.map_err(|error| error.to_string())
    }
}
