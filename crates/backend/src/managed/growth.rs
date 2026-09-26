//! Growth of a Cloud's filesystems (`managed-hosts.md` § Lifecycle and
//! capacity): the Cloud's runner asks for a larger system or home
//! filesystem, the backend holds the request to its policy's maximum, and
//! the machine manager grows the one working filesystem in the device's
//! order, so it never overlaps a save or a reset.

use demi_machines_protocol::{GrowVolumeParams, Volume};
use demi_runner_protocol::wire::VolumeName;
use demi_web_api::devices::DeviceKind;
use demi_web_api::ids::DeviceId;

use crate::shard::Shard;

impl Shard {
    /// Grows `volume` of the Cloud `device` to `bytes`, which must be more
    /// than none and at most its maximum.
    pub(crate) async fn grow_cloud_volume(&self, device: &DeviceId, volume: VolumeName, bytes: u64) -> Result<(), String> {
        let tuning = &self.services().cloud.tuning;
        let (volume, quota) = match volume {
            VolumeName::System => (Volume::System, tuning.system_quota),
            VolumeName::Home => (Volume::Home, tuning.home_quota),
        };
        let Some(bytes) = std::num::NonZeroU64::new(bytes).filter(|bytes| bytes.get() <= quota) else {
            return Err(format!("{volume} volume quota exceeded"));
        };
        let record = self
            .services()
            .control
            .device(device.clone())
            .await
            .map_err(|error| error.to_string())?;
        if !record.is_some_and(|record| record.kind == DeviceKind::Managed && record.user == *self.user()) {
            return Err("Only the Cloud grows its volumes".into());
        }
        let grow = GrowVolumeParams {
            device_id: device.to_string(),
            volume,
            bytes,
        };
        self.services()
            .cloud
            .machines
            .call(grow)
            .await
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use demi_runner_protocol::wire::{RunnerPlatform, VolumeName};

    use crate::auth::sessions::TokenHash;
    use crate::backend::Services;
    use crate::shard::{ShardPlacement, ShardPool};
    use crate::storage::control::testing;

    #[tokio::test(flavor = "local")]
    async fn a_growth_is_more_than_nothing_within_its_volumes_maximum_and_only_for_the_cloud() {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        let control = services.control.clone();
        let owner = testing::master(&control).await.id;
        let cloud = control.managed_device_or_create(owner.clone()).await.unwrap().id;
        let laptop = control
            .create_device(owner.clone(), "laptop".into(), RunnerPlatform::Darwin, TokenHash::of("laptop"))
            .await
            .unwrap()
            .id;
        let tuning = services.cloud.tuning;
        let pool = ShardPool::start(ShardPlacement::Inline, services).await.unwrap();
        let answers = pool
            .shards()
            .of(&owner)
            .call(move |shard, _| async move {
                let mut answers = Vec::new();
                for (device, volume, bytes) in [
                    (&cloud, VolumeName::System, 0),
                    (&cloud, VolumeName::System, tuning.system_quota + 1),
                    (&cloud, VolumeName::Home, tuning.home_quota + 1),
                    (&laptop, VolumeName::Home, 1 << 30),
                    (&cloud, VolumeName::Home, tuning.home_quota),
                ] {
                    answers.push(shard.grow_cloud_volume(device, volume, bytes).await);
                }
                answers
            })
            .await
            .unwrap();
        assert_eq!(answers[0], Err("system volume quota exceeded".into()));
        assert_eq!(answers[1], Err("system volume quota exceeded".into()));
        assert_eq!(answers[2], Err("home volume quota exceeded".into()));
        assert_eq!(answers[3], Err("Only the Cloud grows its volumes".into()));
        // Within its maximum, the Cloud's growth goes to the manager, which
        // no test service runs.
        let reached = answers[4].clone().unwrap_err();
        assert!(reached.starts_with("Machine manager unavailable during grow_volume"), "{reached}");
        pool.close().await;
    }
}
