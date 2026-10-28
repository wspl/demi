//! The deletion of a managed machine, an integration's Cloud, and of an
//! integration's shared folder.
//!
//! A Cloud is deleted once no conversation works on it: its runner goes,
//! which ends every command still running there, the operations that hold
//! it end, it stops, and the manager removes its storage. Its device record
//! is the caller's to delete after.

use demi_backend_database::devices::DeviceRecord;
use demi_machine_manager_protocol::{DeleteParams, DeleteSharedParams};

use crate::CloudShard;
use crate::machine::CloudError;

impl dyn CloudShard {
    /// Deletes the machine of `device`, which no conversation uses.
    pub async fn delete_machine(&self, device: &DeviceRecord) -> Result<(), CloudError> {
        let machine = self.cloud_machine(device).await?;
        if machine.resetting().is_some() {
            return Err(CloudError::Resetting);
        }
        self.devices().disconnect(&device.id, "Cloud is being deleted");
        let hold = self.cloud_services().tuning.reset_hold;
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
        Ok(())
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
