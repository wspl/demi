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
        let path = write_attachment(&host.host, id, file_name, bytes.clone()).await?;
        let upload = Upload {
            name: file_name,
            path: &path,
            media_type: &record.media_type,
            sha256: &record.sha256,
            bytes: &B64Bytes::from(bytes),
        };
        Ok(upload_blocks(upload, &blobs).await?)
    }
}

/// Writes `bytes` into the conversation's attachment directory on `host`,
/// under `file_name` or, when a file of other bytes has that name already,
/// the first free `name-2.ext`, `name-3.ext` and so on, which the write
/// itself takes (`runner.md` § Host operations), and answers the absolute
/// path. A file of the same bytes under one of those names is the
/// attachment already there, and is not written again (`product.md`
/// § Attachments). Nothing is overwritten, and what is written stays, so the
/// path a transcript names stays readable. The number belongs to the path
/// alone: the attachment keeps `file_name`.
async fn write_attachment(
    host: &RemoteHost,
    id: &ConversationId,
    file_name: &str,
    bytes: Bytes,
) -> Result<String, HostError> {
    let identity = host.identity();
    let paths = identity.paths;
    let directory = paths.join(&identity.home_dir, &format!("{ATTACHMENTS_DIR}/{id}"));
    let options = WriteOptions {
        exists: WhenExists::Rename,
    };
    let name = host
        .fs()
        .write_file(
            &paths.join(&directory, file_name),
            FileContents::Bytes(bytes),
            options,
        )
        .await?;
    Ok(paths.join(&directory, &name))
}

#[cfg(test)]
mod tests {
    use demi_backend_remote_host::Admission;
    use demi_backend_remote_host::testing::{
        CommandPolicy, FixtureOptions, RunnerFixture, TEST_DEVICE, TestDevice, answered_requests,
    };
    use demi_host_interface::{CommandSet, HostIdentity, HostPaths};
    use demi_runner_protocol::wire::{FsOk, FsResult, Inbound, Outbound, ReadAt};
    use tokio::sync::mpsc;

    use super::*;

    /// A Windows Host's attachment is written, and named to the model, in
    /// that Host's own form (`runner.md` § Windows paths): before, it was
    /// `C:\Users\ana/.demi/attachments/<id>/notes.txt`. The test is the
    /// runner, which answers the one write. Instant.
    #[tokio::test(flavor = "local")]
    async fn an_attachment_on_a_windows_host_is_written_and_named_in_its_own_form() {
        let device = TestDevice::new(CommandPolicy::new(CommandSet::new())).with_identity(
            HostIdentity {
                uid: 0,
                gid: 0,
                hostname: "DESKTOP-4KQ7M2A".into(),
                home_dir: r"C:\Users\ana".into(),
                paths: HostPaths::Windows,
            },
        );
        let mut link = device.connect(None);
        let host = device.host(r"C:\Users\ana", Admission::Free);
        let id = ConversationId::try_from("0193f1e2-7c4d-7e8f-9a0b-1c2d3e4f5a6b").unwrap();
        let written = tokio::task::spawn_local(async move {
            write_attachment(&host, &id, "notes.txt", Bytes::from("notes")).await
        });
        let Inbound::FsWriteFile { id, path, input, .. } = link.next().await else {
            panic!("expected a write request")
        };
        let directory = r"C:\Users\ana\.demi\attachments\0193f1e2-7c4d-7e8f-9a0b-1c2d3e4f5a6b";
        assert_eq!(path, format!(r"{directory}\notes.txt"));
        // The runner takes the bytes, then names the free name it wrote.
        let sink = device.pipes().claim_sink(&input.id, TEST_DEVICE).unwrap();
        drop(sink);
        link.send(Outbound::FsOk(FsOk {
            id,
            read_at: ReadAt {
                epoch: "e".into(),
                number: 1,
            },
            result: FsResult::WriteFile("notes-2.txt".into()),
        }))
        .await;
        assert_eq!(
            written.await.unwrap().unwrap(),
            format!(r"{directory}\notes-2.txt")
        );
    }

    // About a tenth of a second: a real runner process writes the files.
    #[tokio::test(flavor = "local")]
    async fn an_attachment_of_other_bytes_takes_the_first_free_name_and_the_same_one_is_reused() {
        let (tap, mut replies) = mpsc::channel(1 << 10);
        let fixture = RunnerFixture::start(FixtureOptions {
            tap: Some(tap),
            ..FixtureOptions::default()
        })
        .await;
        let host = fixture.host();
        let id = ConversationId::try_from("0193f1e2-7c4d-7e8f-9a0b-1c2d3e4f5a6b").unwrap();
        answered_requests(&mut replies);

        // Sent again, pasted or put back by Undo Send, the same file is the
        // one already there: its path, and not a byte written.
        let mut paths = Vec::new();
        let mut written = Vec::new();
        for text in ["one", "two", "one", "two", "three"] {
            let path = write_attachment(&host, &id, "notes.txt", Bytes::from(text))
                .await
                .unwrap();
            assert_eq!(answered_requests(&mut replies), 1, "one request writes {text}");
            assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
            let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
            if let Some((_, first)) = written.iter().find(|(seen, _)| *seen == path) {
                assert_eq!(*first, modified, "{text} is written again");
            } else {
                written.push((path.clone(), modified));
            }
            paths.push(path.rsplit('/').next().unwrap().to_owned());
        }
        assert_eq!(
            paths,
            ["notes.txt", "notes-2.txt", "notes.txt", "notes-2.txt", "notes-3.txt"]
        );
        let directory = std::path::Path::new(&written[0].0).parent().unwrap();
        assert_eq!(std::fs::read_dir(directory).unwrap().count(), 3);
        fixture.stop().await;
    }
}
