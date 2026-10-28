//! The deletion of a managed machine, an integration's Cloud, and of an
//! integration's shared folder.
//!
//! A Cloud is deleted as it is reset: the turns of the conversations that
//! work on it are interrupted, its runner goes, which ends every command
//! still running there, the operations that hold it end, it stops, and the
//! manager removes its storage. Its device record is the caller's to delete
//! after.

use demi_backend_database::devices::DeviceRecord;
use demi_machine_manager_protocol::{DeleteParams, DeleteSharedParams};

use crate::machine::CloudError;
use crate::{CloudShard, ConversationHold};

/// The conversations that worked on a deleted machine, held until the
/// caller has deleted its device too, so no turn of theirs reaches it
/// between.
#[must_use]
pub struct DeletedMachine {
    _held: Vec<Box<dyn ConversationHold>>,
}

impl dyn CloudShard {
    /// Deletes the machine of `device`, holding the conversations that work
    /// on it until the answer is dropped.
    pub async fn delete_machine(&self, device: &DeviceRecord) -> Result<DeletedMachine, CloudError> {
        let machine = self.cloud_machine(device).await?;
        if machine.resetting().is_some() {
            return Err(CloudError::Resetting);
        }
        let hold = self.cloud_services().tuning.reset_hold;
        let uses = self.cloud_uses(device).await?;
        let _held = self.hold_uses(&uses, hold, "deletion").await?;
        self.devices().disconnect(&device.id, "Cloud is being deleted");
        let _reserved = tokio::time::timeout(hold, machine.gate.reserve())
            .await
            .map_err(|_| CloudError::Failed("The Cloud's operations did not end for its deletion".into()))?;
        self.hibernate_reserved(&machine).await?;
        self.cloud_services()
            .machines
            .call(DeleteParams {
                device_id: device.id.to_string(),
            })
            .await?;
        self.cloud().forget(&device.id);
        Ok(DeletedMachine { _held })
    }

    /// Removes the shared folder `id`, once none of its Clouds runs.
    pub async fn delete_shared_folder(&self, id: &str) -> Result<(), CloudError> {
        self.cloud_services()
            .machines
            .call(DeleteSharedParams { id: id.to_owned() })
            .await?;
        Ok(())
    }
}
