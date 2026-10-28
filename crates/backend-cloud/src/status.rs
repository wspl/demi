//! The Cloud's status as the page reads it: the device, its lifecycle state,
//! its latest reset, why it last failed, its filesystems' capacities, and
//! whether its system is on another image than the configured one. Reading it
//! never wakes the Cloud and asks no one: the machine manager pushes the
//! sizes and the image, and the page shows what it last said.

use demi_backend_database::StorageError;
use demi_backend_database::devices::DeviceRecord;
use demi_backend_database::managed::ManagedOperation;
use demi_machine_manager_protocol::DeviceId;
use demi_web_api_protocol::cloud::{
    CloudDevice, CloudOperation, CloudState, CloudStatus, CloudVolumes,
};

use crate::CloudShard;

impl dyn CloudShard {
    pub async fn cloud_status(&self) -> Result<CloudStatus, StorageError> {
        let Some(device) = self.control().managed_device(self.user().clone()).await? else {
            return Ok(CloudStatus {
                device: None,
                state: CloudState::Unallocated,
                operation: None,
                error: None,
                volumes: None,
                newer_image: false,
            });
        };
        let machine = self.cloud_machine(&device).await?;
        // What the manager last said, which it pushes: reading it calls no
        // one, so an unreachable manager shows the last known sizes.
        let image = DeviceId::parse(device.id.as_str())
            .ok()
            .and_then(|id| self.cloud_services().machines.status(&id));
        let volumes = image.and_then(|image| image.volumes).map(|volumes| CloudVolumes {
            system_bytes: volumes.system_bytes.get(),
            home_bytes: volumes.home_bytes.get(),
        });
        let newer_image = image.is_some_and(|image| image.newer_image);
        Ok(CloudStatus {
            device: Some(CloudDevice {
                id: device.id.clone(),
                name: device.name.clone(),
            }),
            state: machine.state(),
            operation: machine.operation.borrow().as_ref().map(operation_dto),
            error: machine.error.borrow().clone(),
            volumes,
            newer_image,
        })
    }
}

impl dyn CloudShard {
    /// The state of the user's Cloud `device` as the shard knows it, read
    /// without waking it.
    pub async fn machine_state(&self, device: &DeviceRecord) -> Result<CloudState, StorageError> {
        Ok(self.cloud_machine(device).await?.state())
    }
}

/// A reset as the page sees it.
pub fn operation_dto(operation: &ManagedOperation) -> CloudOperation {
    CloudOperation {
        id: operation.id.clone(),
        phase: operation.phase,
        error: operation.error.clone(),
    }
}
