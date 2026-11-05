//! The Cloud's status as the page reads it: the device, its lifecycle state,
//! its latest reset, why it last failed, its filesystems' capacities, and
//! whether its system is on another image than the configured one. Reading it
//! never wakes the Cloud and asks no one: the machine manager pushes the
//! sizes and the image, and the page shows what it last said.

use demi_backend_database::StorageError;
use demi_backend_database::managed::ManagedOperation;
use demi_machine_manager_protocol::{DeviceId, Isolation as MachineIsolation, MoveStatus};
use demi_web_api_protocol::cloud::{
    CloudDevice, CloudMoveStatus, CloudOperation, CloudState, CloudStatus, CloudVolumes,
};
use demi_web_api_protocol::settings::Isolation;

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
                move_status: None,
            });
        };
        let machine = self.cloud_machine(&device).await?;
        // What the manager last said, which it pushes: reading it calls no
        // one, so an unreachable manager shows the last known sizes.
        let image = DeviceId::parse(device.id.as_str())
            .ok()
            .and_then(|id| self.cloud_services().machines.status(&id));
        let volumes = image.as_ref().and_then(|image| image.volumes).map(|volumes| CloudVolumes {
            system_bytes: volumes.system_bytes.get(),
            home_bytes: volumes.home_bytes.get(),
        });
        let newer_image = image.as_ref().is_some_and(|image| image.newer_image);
        let move_status = image.and_then(|image| {
            // The move is to the isolation the Cloud's storage is not on.
            let to = match image.isolation {
                MachineIsolation::Simple => Isolation::Gvisor,
                MachineIsolation::Gvisor => Isolation::Simple,
            };
            Some(match image.move_status? {
                MoveStatus::Moving => CloudMoveStatus::Moving { to },
                MoveStatus::Failed { message } => CloudMoveStatus::Failed { to, message },
            })
        });
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
            move_status,
        })
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
