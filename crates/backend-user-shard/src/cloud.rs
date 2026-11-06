//! The routing of the machine manager's events to the cores of the Clouds'
//! owners.

use std::sync::Arc;

use demi_backend_cloud::user_core::CloudOfCore;
use demi_web_api_protocol::ids::DeviceId;
use tokio::sync::mpsc;

use crate::pool::Shards;
use crate::services::Services;

/// Routes each death the machine manager reports to the shard of the
/// device's owner (`crates/backend/src/lib.rs`), until the backend closes.
pub async fn route_deaths(
    mut deaths: mpsc::Receiver<demi_machine_manager_protocol::DeviceId>,
    services: Arc<Services>,
    shards: Shards,
) {
    while let Some(device) = deaths.recv().await {
        let Ok(id) = DeviceId::try_from(device.as_str()) else {
            continue;
        };
        let owner = match services.control.device_owner(id.clone()).await {
            Ok(Some(owner)) => owner,
            // A device the backend no longer has is no one's to stop.
            Ok(None) => continue,
            Err(error) => {
                tracing::error!(event = "cloud.death_unrouted", host_id = %id, %error, "the death of a Cloud could not be routed");
                continue;
            }
        };
        // A shard that is closing has no Cloud left to stop.
        let _ = shards
            .of(&owner)
            .call(move |shard, _| async move { shard.cloud_shard().cloud_died(&id).await })
            .await;
    }
}

/// Marks the Cloud part of the owner's pages changed for each device the
/// machine manager reports a new status of, until the backend closes.
pub async fn route_statuses(
    mut statuses: mpsc::Receiver<demi_machine_manager_protocol::DeviceId>,
    services: Arc<Services>,
    shards: Shards,
) {
    while let Some(device) = statuses.recv().await {
        let Ok(id) = DeviceId::try_from(device.as_str()) else {
            continue;
        };
        let owner = match services.control.device_owner(id.clone()).await {
            Ok(Some(owner)) => owner,
            // A device the backend no longer has shows on no page.
            Ok(None) => continue,
            Err(error) => {
                tracing::error!(event = "cloud.status_unrouted", host_id = %id, %error, "a Cloud's new status could not be routed");
                continue;
            }
        };
        // A shard that is closing has no page left to tell.
        let _ = shards
            .of(&owner)
            .call(move |shard, _| async move { shard.cloud_shard().marks().mark(demi_core_sync::Part::Cloud) })
            .await;
    }
}
