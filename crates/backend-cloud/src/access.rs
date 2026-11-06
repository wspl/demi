//! Machine access (`crates/core-host-access/src/lib.rs`): the user's Cloud
//! itself, for work that needs it rather than a conversation's files on it,
//! such as making a Cloud project's directory.
//!
//! It takes the Cloud's admission, which wakes a stopped Cloud or waits for a
//! running reset, and no conversation's file gate.

use demi_core_store::StorageError;
use demi_core_store::devices::{DeviceRecord, ManagedMachine};
use demi_core_sync::Part;

use crate::CloudShard;
use crate::machine::CloudError;
use demi_core_host_access::cloud::MachineAccess;

impl dyn CloudShard {
    /// The user's Cloud device, made on its first use, which the user's
    /// pages then show among the devices and as the Cloud.
    pub async fn cloud_device(&self) -> Result<DeviceRecord, StorageError> {
        let store = self.store();
        if let Some(device) = store.managed_device().await? {
            return Ok(device);
        }
        let device = store.managed_device_or_create().await?;
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
    /// stopped, for the shard of the instance's master, whose record it
    /// is.
    pub async fn claude_code_access(&self) -> Result<MachineAccess, CloudError> {
        let device = self
            .store()
            .managed_machine_or_create(ManagedMachine::ClaudeCode)
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
        Ok(MachineAccess::new(device, host, home, admission))
    }
}
