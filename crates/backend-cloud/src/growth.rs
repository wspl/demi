//! Growth of a Cloud's filesystems (`managed-hosts.md` § Lifecycle and
//! capacity): the Cloud's runner asks for a larger system or home
//! filesystem, the backend holds the request to its policy's maximum, and
//! the machine manager grows the one working filesystem in the device's
//! order, so it never overlaps a save or a reset.

use demi_machine_manager_protocol::{GrowVolumeParams, Volume};
use demi_runner_protocol::wire::VolumeName;
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::ids::DeviceId;

use crate::CloudShard;

impl dyn CloudShard {
    /// Grows `volume` of the Cloud `device` to `bytes`, which must be more
    /// than none and at most its maximum.
    pub async fn grow_cloud_volume(
        &self,
        device: &DeviceId,
        volume: VolumeName,
        bytes: u64,
    ) -> Result<(), String> {
        let tuning = &self.cloud_services().tuning;
        let (volume, quota) = match volume {
            VolumeName::System => (Volume::System, tuning.system_quota),
            VolumeName::Home => (Volume::Home, tuning.home_quota),
        };
        let Some(bytes) = std::num::NonZeroU64::new(bytes).filter(|bytes| bytes.get() <= quota)
        else {
            return Err(format!("{volume} volume quota exceeded"));
        };
        let record = self
            .control()
            .device(device.clone())
            .await
            .map_err(|error| error.to_string())?;
        if !record
            .is_some_and(|record| record.kind == DeviceKind::Managed && record.user == *self.user())
        {
            return Err("Only the Cloud grows its volumes".into());
        }
        let grow = GrowVolumeParams {
            device_id: device.to_string(),
            volume,
            bytes,
        };
        self.cloud_services()
            .machines
            .call(grow)
            .await
            .map_err(|error| error.to_string())
    }
}
