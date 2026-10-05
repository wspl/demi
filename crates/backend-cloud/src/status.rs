//! The Cloud's status as the page reads it (`web-api.md` § Cloud): the
//! device, its lifecycle state, its latest reset, why it last failed, its
//! filesystems' capacities and their limits, and whether its system is on
//! another image than the configured one. Reading it never wakes the Cloud.

use demi_backend_database::StorageError;
use demi_backend_database::managed::ManagedOperation;
use demi_machine_manager_protocol::{CurrentBaseVersionParams, ImageStateParams};
use demi_web_api_protocol::cloud::{
    CloudDevice, CloudOperation, CloudState, CloudStatus, CloudVolumes,
};

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
                newer_image: false,
            });
        };
        let machine = self.cloud_machine(&device).await?;
        let image = ImageStateParams {
            device_id: device.id.to_string(),
        };
        let machines = &self.cloud_services().machines;
        let image = match machines.call(image).await {
            Ok(image) => image,
            Err(error) => {
                // The status answers without them; the manager answers them
                // again when it can.
                tracing::warn!(device = %device.id, "the Cloud's disks could not be read: {error}");
                None
            }
        };
        let volumes = image.as_ref().map(|image| CloudVolumes {
            system_bytes: image.system_bytes.get(),
            home_bytes: image.home_bytes.get(),
        });
        let newer_image = match &image {
            Some(image) => match machines.call(CurrentBaseVersionParams {}).await {
                Ok(configured) => configured != image.base_version,
                Err(error) => {
                    tracing::warn!("the configured Cloud image could not be read: {error}");
                    false
                }
            },
            None => false,
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
            newer_image,
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
