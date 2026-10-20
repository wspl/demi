//! Machine access (`crates/backend-host-access/src/lib.rs`): the
//! user's Cloud itself, for work that needs it rather than a conversation's
//! files on it, such as making a Cloud project's directory. It takes the
//! Cloud's admission, which wakes a stopped Cloud or waits for a running
//! reset, and no conversation's file gate.

use demi_backend_database::StorageError;
use demi_backend_database::devices::{DeviceRecord, ManagedMachine};
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
        self.access(device).await
    }

    /// The Claude Code machine, made on its first use and woken when it is
    /// stopped, for the shard of the instance's master, whose record it is
    /// (`managed-hosts.md` § The Claude Code machine).
    pub async fn claude_code_access(&self) -> Result<MachineAccess, CloudError> {
        let device = self
            .control()
            .managed_machine_or_create(self.user().clone(), ManagedMachine::ClaudeCode)
            .await?;
        self.access(device).await
    }

    /// The managed machine `device`, woken when it is stopped.
    async fn access(&self, device: DeviceRecord) -> Result<MachineAccess, CloudError> {
        let admission = self.admit_cloud(&device).await?;
        let home = self.devices().home(&device.id).ok_or_else(|| {
            CloudError::Failed("The machine did not report its home directory".into())
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
