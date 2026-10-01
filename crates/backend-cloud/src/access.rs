//! Machine access (`sessions-and-targets.md` § Every way to a Host): the
//! user's Cloud itself, for work that needs it rather than a conversation's
//! files on it, such as making a Cloud project's directory. It takes the
//! Cloud's admission, which wakes a stopped Cloud or waits for a running
//! reset, and no conversation's file gate.

use demi_backend_database::StorageError;
use demi_backend_database::devices::DeviceRecord;
use demi_backend_page_sync::Part;
use demi_backend_remote_host::RemoteHost;

use crate::CloudShard;
use crate::machine::{CloudAdmission, CloudError};

/// The user's Cloud, running, and a Host on it that starts in its home;
/// the Cloud stays admitted while this is held.
pub struct MachineAccess {
    pub device: DeviceRecord,
    pub host: RemoteHost,
    /// The home directory the Cloud's runner reported.
    pub home: String,
    _admission: CloudAdmission,
}

impl dyn CloudShard {
    /// The user's Cloud device, made on its first use, which the user's
    /// pages then show among the devices and as the Cloud.
    pub async fn cloud_device(&self) -> Result<DeviceRecord, StorageError> {
        let control = self.control();
        if let Some(device) = control.managed_device(self.user().clone()).await? {
            return Ok(device);
        }
        let device = control
            .managed_device_or_create(self.user().clone())
            .await?;
        let marks = self.marks();
        marks.mark(Part::Devices);
        marks.mark(Part::Cloud);
        Ok(device)
    }

    /// The user's Cloud, made on its first use and woken when it is stopped.
    pub async fn machine_access(&self) -> Result<MachineAccess, CloudError> {
        let device = self.cloud_device().await?;
        let admission = self.admit_cloud(&device).await?;
        let home = self.devices().home(&device.id).ok_or_else(|| {
            CloudError::Failed("The Cloud did not report its home directory".into())
        })?;
        let host =
            self.devices()
                .machine_host(&device.id, home.clone(), admission.per_operation.clone());
        Ok(MachineAccess {
            device,
            host,
            home,
            _admission: admission,
        })
    }
}
