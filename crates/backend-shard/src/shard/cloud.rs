//! The shard's side of the user's Cloud (`managed-hosts.md`): what the
//! Cloud needs of the shard (`CloudShard`), with the holds of the user's
//! conversations an idle stop and a reset take, and the routing of the
//! machine manager's death events to the shards.

use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use demi_backend_cloud::machine::Cloud;
use demi_backend_cloud::{CloudServices, CloudShard, ConversationHold};
use demi_backend_idle::Activity;
use demi_backend_providers::llm::assembly::ProviderAssembly;
use demi_backend_providers::vault::entries::Vault;
use demi_backend_runners::devices::Devices;
use demi_backend_runners::public_url::PublicUrl;
use demi_backend_storage::control::ControlService;
use demi_backend_sync::UserMarks;
use demi_gates::Reservation;
use demi_web_api::ids::{ConversationId, DeviceId, UserId};
use futures_util::future::LocalBoxFuture;
use tokio::sync::mpsc;
use tokio_util::sync::WaitForCancellationFuture;
use tokio_util::task::TaskTracker;

use super::{Shard, Shards};
use crate::services::Services;
use demi_backend_host_access::root_of;
use demi_backend_host_access::transfer::TransfersClosed;

impl CloudShard for Shard {
    fn user(&self) -> &UserId {
        &self.user
    }

    fn cloud(&self) -> &Cloud {
        &self.cloud
    }

    fn devices(&self) -> &Devices {
        &self.devices
    }

    fn control(&self) -> &ControlService {
        &self.services.control
    }

    fn cloud_services(&self) -> &CloudServices {
        &self.services.cloud
    }

    fn public_url(&self) -> &PublicUrl {
        &self.services.public_url
    }

    fn idle_window(&self) -> Duration {
        self.services.lifecycle.idle_window
    }

    fn marks(&self) -> UserMarks {
        self.services.sync.of(&self.user)
    }

    fn vault(&self) -> &Vault {
        &self.services.vault
    }

    fn assembly(&self) -> &ProviderAssembly {
        &self.services.assembly
    }

    fn tasks(&self) -> &TaskTracker {
        &self.tasks
    }

    fn closed(&self) -> WaitForCancellationFuture<'_> {
        self.closing.cancelled()
    }

    fn this(&self) -> Rc<dyn CloudShard> {
        self.this
            .upgrade()
            .expect("a shard's method runs while the shard lives")
    }

    fn activity(&self, conversation: &ConversationId) -> Activity {
        self.conversation_activity(conversation)
    }

    fn attended(&self, conversation: &ConversationId) -> bool {
        let turning = self
            .agent
            .tree(&root_of(conversation))
            .is_some_and(|tree| tree.admission().state().demand > 0);
        turning || self.conversations.slot(conversation).transfers.any_open()
    }

    fn hold_for_idle(&self, conversation: &ConversationId) -> Option<Box<dyn ConversationHold>> {
        let tree = match self.agent.tree(&root_of(conversation)) {
            Some(tree) => Some(tree.admission().try_reserve()?),
            None => None,
        };
        let files = self
            .conversations
            .slot(conversation)
            .file_gate()
            .try_reserve()?;
        let held: Box<dyn ConversationHold> = Box::new(IdleHold {
            _tree: tree,
            _files: files,
        });
        Some(held)
    }

    fn hold_for_reset<'a>(
        &'a self,
        conversation: &'a ConversationId,
        files_on_cloud: bool,
        hold: Duration,
    ) -> LocalBoxFuture<'a, Option<Box<dyn ConversationHold>>> {
        Box::pin(async move {
            let tree = match self.agent.tree(&root_of(conversation)) {
                Some(tree) => Some(tokio::time::timeout(hold, tree.interrupt()).await.ok()?),
                None => None,
            };
            let slot = self.conversations.slot(conversation);
            let transfers = if files_on_cloud {
                Some(slot.transfers.close().await)
            } else {
                None
            };
            let files = tokio::time::timeout(hold, slot.file_gate().reserve())
                .await
                .ok()?;
            let held: Box<dyn ConversationHold> = Box::new(ResetHold {
                _files: files,
                _transfers: transfers,
                _tree: tree,
            });
            Some(held)
        })
    }

    fn cloud_stopped<'a>(&'a self, device: &'a DeviceId) -> LocalBoxFuture<'a, ()> {
        Box::pin(self.expose_shard().destroy_exposes_on(device))
    }
}

/// A conversation an idle stop of the Cloud holds: its tree and its file
/// gate reserved. The fields drop in order, the tree first.
struct IdleHold {
    _tree: Option<Reservation>,
    _files: Reservation,
}

impl ConversationHold for IdleHold {}

/// A conversation a reset holds: its file gate reserved, its transfers and
/// streams ended if its files are on the Cloud, and its tree interrupted.
/// The fields drop in order, the file gate first.
struct ResetHold {
    _files: Reservation,
    _transfers: Option<TransfersClosed>,
    _tree: Option<Reservation>,
}

impl ConversationHold for ResetHold {}

impl Shard {
    /// The shard as its Cloud sees it, whose operations it is.
    pub fn cloud_shard(&self) -> &(dyn CloudShard + 'static) {
        self
    }
}

/// Routes each death the machine manager reports to the shard of the
/// device's owner (`backend.md` § Runtime model), until the backend closes.
pub async fn route_deaths(
    mut deaths: mpsc::Receiver<demi_machines_protocol::DeviceId>,
    services: Arc<Services>,
    shards: Shards,
) {
    while let Some(device) = deaths.recv().await {
        let Ok(id) = DeviceId::try_from(device.as_str()) else {
            continue;
        };
        let owner = match services.control.device(id.clone()).await {
            Ok(Some(record)) => record.user,
            // A device the backend no longer has is no one's to stop.
            Ok(None) => continue,
            Err(error) => {
                tracing::error!(device = %id, "the death of a Cloud could not be routed: {error}");
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

#[cfg(test)]
mod tests {
    use demi_backend_storage::accounts::TokenHash;
    use demi_backend_storage::control::testing;
    use demi_runner_protocol::wire::{RunnerPlatform, VolumeName};

    use crate::services::Services;
    use crate::shard::{ShardPlacement, ShardPool};

    #[tokio::test(flavor = "local")]
    async fn a_growth_is_more_than_nothing_within_its_volumes_maximum_and_only_for_the_cloud() {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        let control = services.control.clone();
        let owner = testing::master(&control).await.id;
        let cloud = control
            .managed_device_or_create(owner.clone())
            .await
            .unwrap()
            .id;
        let laptop = control
            .create_device(
                owner.clone(),
                "laptop".into(),
                RunnerPlatform::Darwin,
                TokenHash::of("laptop"),
            )
            .await
            .unwrap()
            .id;
        let tuning = services.cloud.tuning;
        let pool = ShardPool::start(ShardPlacement::Inline, services)
            .await
            .unwrap();
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
                    answers.push(
                        shard
                            .cloud_shard()
                            .grow_cloud_volume(device, volume, bytes)
                            .await,
                    );
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
        assert!(
            reached.starts_with("Machine manager unavailable during grow_volume"),
            "{reached}"
        );
        pool.close().await;
    }
}
