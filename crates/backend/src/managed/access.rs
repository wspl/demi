//! Machine access (`sessions-and-targets.md` § Every way to a Host): the
//! user's Cloud itself, for work that needs it rather than a conversation's
//! files on it, such as making a Cloud project's directory. It takes the
//! Cloud's admission, which wakes a stopped Cloud or waits for a running
//! reset, and no conversation's file gate.

use demi_host_remote::RemoteHost;

use super::machine::{CloudAdmission, CloudError};
use crate::runner::{HostOwner, host_key};
use crate::shard::Shard;
use crate::storage::devices::DeviceRecord;

/// The user's Cloud, running, and a Host on it that starts in its home;
/// the Cloud stays admitted while this is held.
pub(crate) struct MachineAccess {
    pub(crate) device: DeviceRecord,
    pub(crate) host: RemoteHost,
    /// The home directory the Cloud's runner reported.
    pub(crate) home: String,
    _admission: CloudAdmission,
}

impl Shard {
    /// The user's Cloud, made on its first use and woken when it is stopped.
    pub(crate) async fn machine_access(&self) -> Result<MachineAccess, CloudError> {
        let device = self
            .services()
            .control
            .managed_device_or_create(self.user().clone())
            .await?;
        let admission = self.admit_cloud(&device).await?;
        let home = self
            .devices()
            .home(&device.id)
            .ok_or_else(|| CloudError::Failed("The Cloud did not report its home directory".into()))?;
        let key = host_key(&device.id, HostOwner::MachineAccess, &home);
        let host = self
            .devices()
            .host(&device.id, key, home.clone(), admission.per_operation.clone());
        Ok(MachineAccess {
            device,
            host,
            home,
            _admission: admission,
        })
    }
}
