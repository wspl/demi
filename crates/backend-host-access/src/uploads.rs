//! The uploads a frame's content names (`backend.md` § Media by reference):
//! an upload of the conversation's owner is written to the conversation's
//! Host under `~/.demi/attachments/<conversation>/`, outside every working
//! directory, and becomes its native media block, when it has one, then its
//! attachment record; the medium references the upload's own blob, or the
//! blob of the image fitted from it, which is stored here, and its bytes go
//! to the session with it. An upload that is gone, another user's, or whose
//! bytes are missing becomes the text that says it is not available.

use bytes::Bytes;
use demi_agent_store::attachments::{Upload, unavailable, upload_blocks};
use demi_agent_store::media::HeldMedia;
use demi_backend_remote_host::RemoteHost;
use demi_host_interface::{FileContents, Host as _, HostError, WhenExists, WriteOptions};
use demi_shared_types::{B64Bytes, UserContentBlock};
use demi_web_api_protocol::ids::{AttachmentId, ConversationId};

use crate::HostShard;
use crate::access::{ConversationHost, HostAccessError};

/// Where a Host keeps the conversations' attachments, under its account's
/// home.
const ATTACHMENTS_DIR: &str = ".demi/attachments";

impl dyn HostShard + '_ {
    /// The blocks the upload `reference` becomes in a message of the user's
    /// conversation `id`, written under `file_name` on `host`, the Host the
    /// frame was admitted on, with the bytes of its medium.
    pub async fn resolve_upload(
        &self,
        id: &ConversationId,
        host: &ConversationHost,
        reference: &str,
        file_name: &str,
    ) -> Result<(Vec<UserContentBlock>, HeldMedia), HostAccessError> {
        let not_available = || Ok((vec![unavailable(reference)], HeldMedia::default()));
        let Ok(attachment) = AttachmentId::try_from(reference) else {
            return not_available();
        };
        let record = self.control().attachment(attachment).await?;
        let Some(record) = record.filter(|record| record.owner == *self.user()) else {
            return not_available();
        };
        let blobs = self.blobs();
        let Some(bytes) = blobs.get(&record.sha256).await? else {
            return not_available();
        };
        let written = write_attachment(&host.host, id, file_name, bytes.clone()).await?;
        let upload = Upload {
            name: &written.name,
            path: &written.path,
            media_type: &record.media_type,
            sha256: &record.sha256,
            bytes: &B64Bytes::from(bytes),
        };
        Ok(upload_blocks(upload, &blobs).await?)
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
/// `name-2.ext`, `name-3.ext` and so on, which the write itself takes
/// (`runner.md` § Host operations); nothing is overwritten, and what is
/// written stays, so the path a transcript names stays readable.
async fn write_attachment(
    host: &RemoteHost,
    id: &ConversationId,
    file_name: &str,
    bytes: Bytes,
) -> Result<Written, HostError> {
    let home = host.identity().home_dir;
    let directory = format!("{}/{ATTACHMENTS_DIR}/{id}", home.trim_end_matches('/'));
    let options = WriteOptions {
        exists: WhenExists::Rename,
    };
    let name = host
        .fs()
        .write_file(
            &format!("{directory}/{file_name}"),
            FileContents::Bytes(bytes),
            options,
        )
        .await?;
    let path = format!("{directory}/{name}");
    Ok(Written { name, path })
}

#[cfg(test)]
mod tests {
    use demi_backend_remote_host::testing::{FixtureOptions, RunnerFixture, answered_requests};
    use tokio::sync::mpsc;

    use super::*;

    // About a tenth of a second: a real runner process writes the files.
    #[tokio::test(flavor = "local")]
    async fn an_attachment_takes_the_first_free_name_with_one_request() {
        let (tap, mut replies) = mpsc::channel(1 << 10);
        let fixture = RunnerFixture::start(FixtureOptions {
            tap: Some(tap),
            ..FixtureOptions::default()
        })
        .await;
        let host = fixture.host();
        let id = ConversationId::try_from("0193f1e2-7c4d-7e8f-9a0b-1c2d3e4f5a6b").unwrap();
        answered_requests(&mut replies);

        let mut names = Vec::new();
        for text in ["one", "two", "three"] {
            let written = write_attachment(&host, &id, "notes.txt", Bytes::from(text))
                .await
                .unwrap();
            assert_eq!(answered_requests(&mut replies), 1, "one request writes {text}");
            assert_eq!(std::fs::read_to_string(&written.path).unwrap(), text);
            names.push(written.name);
        }
        assert_eq!(names, ["notes.txt", "notes-2.txt", "notes-3.txt"]);
        fixture.stop().await;
    }
}
