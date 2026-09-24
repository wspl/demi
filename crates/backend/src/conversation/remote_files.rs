//! Files on the user's devices that a message refers to (`web-api.md`
//! § Device files and remote references): every referenced device must be
//! the user's and connected before any is granted; a device that is not the
//! conversation's main Host is attached, which the conversation's nodes hear
//! of at their next context block; and each reference keeps its device's
//! identity and the shell-quoted `demi host shell --host` command that reads
//! the file when the agent runs it. A reference is no snapshot of the
//! file's bytes: a device revoked or disconnected before the read makes the
//! command fail.

use demi_core::UserContentBlock;
use demi_web_api::ids::{ConversationId, DeviceId};

use crate::shard::Shard;
use crate::storage::StorageError;
use crate::storage::conversation_index::AttachedHostRecord;
use crate::storage::devices::DeviceRecord;

/// A file a message names on one of the user's devices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RemoteFile {
    pub(crate) device: String,
    /// Absolute on the device.
    pub(crate) path: String,
}

/// Why a message's remote files are granted none.
#[derive(Debug, thiserror::Error)]
pub(crate) enum RemoteFileRefusal {
    #[error("Referenced device is not accessible")]
    NotAccessible,
    #[error("Referenced device {0} is offline")]
    Offline(String),
    #[error("A referenced path cannot be written in a shell command")]
    Unquotable,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl Shard {
    /// The reference each of `files` stands for, in their order.
    pub(crate) async fn reference_remote_files(
        &self,
        conversation: &ConversationId,
        files: &[RemoteFile],
    ) -> Result<Vec<UserContentBlock>, RemoteFileRefusal> {
        let control = &self.services().control;
        let record = control
            .conversation(conversation.clone())
            .await?
            .filter(|record| record.owner == *self.user())
            .ok_or(RemoteFileRefusal::NotAccessible)?;
        let mut devices: Vec<DeviceRecord> = Vec::new();
        for file in files {
            let id = DeviceId::try_from(file.device.as_str()).map_err(|_| RemoteFileRefusal::NotAccessible)?;
            if devices.iter().any(|device| device.id == id) {
                continue;
            }
            let device = control
                .device(id)
                .await?
                .filter(|device| device.user == record.owner)
                .ok_or(RemoteFileRefusal::NotAccessible)?;
            if !self.devices().online(&device.id) {
                return Err(RemoteFileRefusal::Offline(device.name));
            }
            devices.push(device);
        }
        let references = files
            .iter()
            .map(|file| {
                let device = devices
                    .iter()
                    .find(|device| device.id.as_str() == file.device)
                    .expect("every referenced device was checked");
                reference(device, &file.path)
            })
            .collect::<Result<Vec<_>, _>>()?;
        let main = self.resolve_target(&record).await?.device().cloned();
        for device in devices {
            if Some(&device.id) == main.as_ref() {
                continue;
            }
            let host = AttachedHostRecord {
                device: device.id,
                name: device.name,
                cwd: None,
            };
            control.attach_host(record.id.clone(), host).await?;
        }
        Ok(references)
    }
}

/// The text a model reads for `path` on `device`: a `file:` URL of the path
/// that names the device and the command that reads the file.
fn reference(device: &DeviceRecord, path: &str) -> Result<UserContentBlock, RemoteFileRefusal> {
    let read = format!("cat -- {}", quote(path)?);
    let command = format!("demi host shell --host {} {}", quote(device.id.as_str())?, quote(&read)?);
    let mut url = url::Url::parse("file:///").expect("the root file URL parses");
    url.set_path(path);
    url.query_pairs_mut()
        .append_pair("host", &device.name)
        .append_pair("deviceId", device.id.as_str())
        .append_pair("readCommand", &command);
    Ok(UserContentBlock::Reference { reference: url.into() })
}

/// `word` as one shell word.
fn quote(word: &str) -> Result<String, RemoteFileRefusal> {
    shlex::try_quote(word)
        .map(|quoted| quoted.into_owned())
        .map_err(|_| RemoteFileRefusal::Unquotable)
}

#[cfg(test)]
mod tests {
    use demi_web_api::ids::UserId;

    use super::*;
    use crate::auth::sessions::TokenHash;
    use crate::backend::Services;
    use crate::shard::{ShardPlacement, ShardPool};
    use crate::storage::control::testing;
    use crate::storage::conversation_index::Creation;

    #[tokio::test(flavor = "local")]
    async fn a_reference_keeps_its_device_and_path_and_an_inaccessible_one_grants_nothing() {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        let control = services.control.clone();
        let owner = testing::master(&control).await.id;
        testing::execute(
            &control,
            "INSERT INTO users (id, email, nickname, password_hash, role, created_at)
             VALUES ('other', 'other@example.test', '', 'unused', 'user', 0)",
            Vec::new(),
        )
        .await;
        let other = UserId::try_from("other").unwrap();
        let conversation = ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01").unwrap();
        assert!(matches!(
            control.create_conversation(owner.clone(), conversation.clone()).await.unwrap(),
            Creation::Created(_)
        ));
        let device = |user: UserId, token: &'static str| {
            let control = control.clone();
            async move {
                control
                    .create_device(user, "build".into(), "linux".into(), TokenHash::of(token))
                    .await
                    .unwrap()
                    .id
            }
        };
        let build = device(owner.clone(), "own").await;
        let foreign = device(other, "foreign").await;
        let pool = ShardPool::start(ShardPlacement::Inline, services).await.unwrap();
        let referenced = build.clone();
        let answers = pool
            .shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let build = referenced;
                let path = "/srv/it's $(literal).txt".to_owned();
                let file = |device: &DeviceId| RemoteFile {
                    device: device.to_string(),
                    path: path.clone(),
                };
                // Offline, the device grants nothing.
                let offline = shard.reference_remote_files(&conversation, &[file(&build)]).await;
                assert!(matches!(offline, Err(RemoteFileRefusal::Offline(_))), "{offline:?}");
                let _connection = shard.connect_for_tests(&build, "/home/build");
                // One inaccessible reference refuses them all.
                let refused = shard
                    .reference_remote_files(&conversation, &[file(&build), file(&foreign)])
                    .await;
                assert!(matches!(refused, Err(RemoteFileRefusal::NotAccessible)), "{refused:?}");
                let attached_before = shard.services().control.attached_hosts(conversation.clone()).await.unwrap();
                let granted = shard.reference_remote_files(&conversation, &[file(&build)]).await.unwrap();
                let attached = shard.services().control.attached_hosts(conversation.clone()).await.unwrap();
                let record = shard.owned_conversation(&conversation).await.unwrap();
                (attached_before, granted, attached, record.context_version, path)
            })
            .await
            .unwrap();
        let (attached_before, granted, attached, context_version, path) = answers;
        assert!(attached_before.is_empty());
        let [UserContentBlock::Reference { reference }] = granted.as_slice() else {
            panic!("expected one reference, got {granted:?}");
        };
        let url = url::Url::parse(reference).unwrap();
        let query: std::collections::HashMap<_, _> = url.query_pairs().into_owned().collect();
        assert_eq!(query["host"], "build");
        assert_eq!(query["deviceId"], build.as_str());
        assert_eq!(
            percent_encoding::percent_decode_str(url.path()).decode_utf8().unwrap(),
            path
        );
        // The command reads the exact path, however the shell would read it.
        let command = shlex::split(&query["readCommand"]).unwrap();
        assert_eq!(command[..5], ["demi", "host", "shell", "--host", build.as_str()]);
        assert_eq!(shlex::split(&command[5]).unwrap(), ["cat", "--", path.as_str()]);
        // The device is attached once, which the nodes hear of.
        assert_eq!(attached.iter().map(|host| &host.device).collect::<Vec<_>>(), [&build]);
        assert_eq!(context_version, 1);
        pool.close().await;
    }
}
