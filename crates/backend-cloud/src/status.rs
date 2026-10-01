//! The Cloud's status as the page reads it (`web-api.md` § Cloud): the
//! device, its lifecycle state, its latest reset, why it last failed, its
//! filesystems' capacities and their limits. Reading it never wakes the
//! Cloud.

use demi_backend_database::StorageError;
use demi_backend_database::managed::ManagedOperation;
use demi_machine_manager_protocol::ImageStateParams;
use demi_web_api_protocol::cloud::{CloudDevice, CloudOperation, CloudState, CloudStatus, CloudVolumes};

use crate::CloudShard;

impl dyn CloudShard {
    pub async fn cloud_status(&self) -> Result<CloudStatus, StorageError> {
        let tuning = &self.cloud_services().tuning;
        let limits = CloudVolumes {
            system_bytes: tuning.system_quota,
            home_bytes: tuning.home_quota,
        };
        let Some(device) = self.control().managed_device(self.user().clone()).await? else {
            return Ok(CloudStatus {
                device: None,
                state: CloudState::Unallocated,
                operation: None,
                error: None,
                volumes: None,
                limits,
            });
        };
        let machine = self.cloud_machine(&device).await?;
        let image = ImageStateParams {
            device_id: device.id.to_string(),
        };
        let volumes = match self.cloud_services().machines.call(image).await {
            Ok(image) => image.map(|image| CloudVolumes {
                system_bytes: image.system_bytes.get(),
                home_bytes: image.home_bytes.get(),
            }),
            Err(error) => {
                // The status answers without them; the manager answers them
                // again when it can.
                tracing::warn!(device = %device.id, "the Cloud's disks could not be read: {error}");
                None
            }
        };
        Ok(CloudStatus {
            device: Some(CloudDevice {
                id: device.id.clone(),
                name: device.name.clone(),
            }),
            state: machine.state(),
            operation: machine.operation.borrow().as_ref().map(operation_dto),
            error: machine.error.borrow().clone(),
            volumes,
            limits,
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
