//! Files on the user's devices that a message refers to (`web-api.md`
//! § Device files and remote references): every referenced device must be
//! the user's and connected before any is granted; a device that is not the
//! conversation's main Host is attached, which the conversation's nodes hear
//! of at their next context block; and each reference keeps its device's
//! identity and the shell-quoted `demi host shell --host` command that reads
//! the file when the agent runs it. A reference is no snapshot of the
//! file's bytes: a device revoked or disconnected before the read makes the
//! command fail.

use demi_backend_storage::StorageError;
use demi_backend_storage::conversation_index::{AttachedHostRecord, ChangeOutcome, RecordChange};
use demi_backend_storage::devices::DeviceRecord;
use demi_core::UserContentBlock;
use demi_web_api::ids::{ConversationId, DeviceId};

use crate::HostShard;

/// A file a message names on one of the user's devices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemoteFile {
    pub device: String,
    /// Absolute on the device.
    pub path: String,
}

/// Why a message's remote files are granted none.
#[derive(Debug, thiserror::Error)]
pub enum RemoteFileRefusal {
    #[error("Referenced device is not accessible")]
    NotAccessible,
    #[error("Referenced device {0} is offline")]
    Offline(String),
    #[error("A referenced path cannot be written in a shell command")]
    Unquotable,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl dyn HostShard + '_ {
    /// The reference each of `files` stands for, in their order.
    pub async fn reference_remote_files(
        &self,
        conversation: &ConversationId,
        files: &[RemoteFile],
    ) -> Result<Vec<UserContentBlock>, RemoteFileRefusal> {
        let control = self.control();
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
            let attached = control
                .change_conversation(record.id.clone(), RecordChange::Attach(host))
                .await?;
            if attached != ChangeOutcome::Applied {
                return Err(RemoteFileRefusal::NotAccessible);
            }
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
