//! The uploads a frame's content names.
//!
//! An upload of the conversation's owner takes the conversation's next
//! attachment number, `a7`, from the sequence `demi attachment upload` takes
//! its numbers from. It becomes its native media block, when it has one, then
//! its attachment record.
//!
//! Nothing is written to a Host, so a slow or broken link to it never holds a
//! send and no stopped Cloud wakes for one: a command that needs the file
//! fetches it with `demi attachment download`.
//!
//! The medium references the upload's own blob, or the blob of the image fitted
//! from it, which is stored here, and its bytes go to the session with it. An
//! upload that is gone, another user's, or whose bytes are missing becomes the
//! text that says it is not available.

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
        let resolved = |outcome: &'static str| {
            tracing::info!(
                event = "attachment.resolved",
                conversation = %id,
                attachment = reference,
                outcome,
                by = "page",
                "an upload a message names was looked up"
            );
        };
        let Some(record) = record.filter(|record| record.owner == *self.user()) else {
            resolved("unknown");
            return not_available();
        };
        resolved("found");
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
            // The user's file is no agent's upload.
            node: None,
            source: None,
        };
        let row = commit(self.tasks(), self.conversation_db(id), numbered, || {}).await?;
        let number = row.number.to_string();
        tracing::info!(
            event = "attachment.stored",
            conversation = %id,
            attachment = number.as_str(),
            bytes = row.size,
            by = "page",
            "an upload was numbered in a conversation"
        );
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
