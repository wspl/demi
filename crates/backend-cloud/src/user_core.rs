//! The Cloud's side of the user's core (`crates/core/src/lib.rs`): what the
//! Cloud needs of the core (`CloudShard`), with what the server keeps in the
//! core for it (`CloudSide`), and the Cloud as the core's Host access sees it
//! (`CloudHost`).

use std::rc::Rc;
use std::time::Duration;

use demi_backend_database::control::ControlService;
use demi_core::shard::Shard;
use demi_core::shard::holds::{IdleHold, ResetHold};
use demi_core_host_access::cloud::{CloudAdmission, CloudError, CloudHost, MachineAccess};
use demi_core_idle_watch::Activity;
use demi_core_providers::vault::entries::Vault;
use demi_core_hosts::devices::Devices;
use demi_core_hosts::public_url::PublicUrl;
use demi_core_store::StorageError;
use demi_core_store::devices::DeviceRecord;
use demi_core_store::store::CoreStore;
use demi_core_sync::UserMarks;
use demi_plugin_host::Lost;
use demi_provider_registry::registry::ProviderRegistry;
use demi_runner_client::Admission;
use demi_web_api_protocol::ids::{ConversationId, DeviceId, ProviderId, UserId};
use futures_util::future::LocalBoxFuture;
use tokio_util::sync::WaitForCancellationFuture;
use tokio_util::task::TaskTracker;

use crate::machine::Cloud;
use crate::{CloudServices, CloudShard, CloudSide, ConversationHold};

impl ConversationHold for IdleHold {}

impl ConversationHold for ResetHold {}

/// What the server keeps in `core` for its Cloud.
fn side(core: &Shard) -> &CloudSide {
    core.extension()
        .expect("the server starts each of its cores with its Cloud's side")
}

impl CloudShard for Shard {
    fn user(&self) -> &UserId {
        Shard::user(self)
    }

    fn cloud(&self) -> &Cloud {
        &side(self).cloud
    }

    fn devices(&self) -> &Devices {
        Shard::devices(self)
    }

    fn control(&self) -> &ControlService {
        &side(self).control
    }

    fn store(&self) -> &CoreStore {
        Shard::store(self)
    }

    fn cloud_services(&self) -> &CloudServices {
        &side(self).services
    }

    fn public_url(&self) -> &PublicUrl {
        &self.services().public_url
    }

    fn idle_window(&self) -> Duration {
        self.services().lifecycle.idle_window
    }

    fn marks(&self) -> UserMarks {
        self.services().sync.of(Shard::user(self))
    }

    fn vault(&self) -> &Vault {
        &self.services().vault
    }

    fn provider_registry(&self) -> &ProviderRegistry {
        &self.services().provider_registry
    }

    fn tasks(&self) -> &TaskTracker {
        Shard::tasks(self)
    }

    fn closed(&self) -> WaitForCancellationFuture<'_> {
        Shard::closed(self)
    }

    fn this(&self) -> Rc<dyn CloudShard> {
        Shard::this(self)
    }

    fn activity(&self, conversation: &ConversationId) -> Activity {
        self.conversation_activity(conversation)
    }

    fn tree_providers(&self, conversation: &ConversationId) -> Vec<ProviderId> {
        Shard::tree_providers(self, conversation)
    }

    fn attended(&self, conversation: &ConversationId) -> bool {
        Shard::attended(self, conversation)
    }

    fn hold_for_idle(&self, conversation: &ConversationId) -> Option<Box<dyn ConversationHold>> {
        let held: Box<dyn ConversationHold> = Box::new(Shard::hold_for_idle(self, conversation)?);
        Some(held)
    }

    fn packages_lost(&self, device: &DeviceId) {
        let core = Shard::this(self);
        let device = device.clone();
        Shard::tasks(self).spawn_local(async move {
            core.package_lost(&device, Lost::Every).await;
        });
    }

    fn hold_for_reset<'a>(
        &'a self,
        conversation: &'a ConversationId,
        files_on_cloud: bool,
        hold: Duration,
    ) -> LocalBoxFuture<'a, Option<Box<dyn ConversationHold>>> {
        Box::pin(async move {
            let held: Box<dyn ConversationHold> =
                Box::new(Shard::hold_for_reset(self, conversation, files_on_cloud, hold).await?);
            Some(held)
        })
    }
}

/// The core as its Cloud sees it, whose operations it is.
pub trait CloudOfCore {
    fn cloud_shard(&self) -> &(dyn CloudShard + 'static);
}

impl CloudOfCore for Shard {
    fn cloud_shard(&self) -> &(dyn CloudShard + 'static) {
        self
    }
}

/// The user's Cloud as the core's Host access sees it.
pub struct UserCloud(pub Rc<Shard>);

impl CloudHost for UserCloud {
    fn cloud_device(&self) -> LocalBoxFuture<'_, Result<DeviceRecord, CloudError>> {
        Box::pin(async move { Ok(self.0.cloud_shard().cloud_device().await?) })
    }

    fn admit_cloud<'a>(&'a self, device: &'a DeviceRecord) -> LocalBoxFuture<'a, Result<CloudAdmission, CloudError>> {
        Box::pin(self.0.cloud_shard().admit_cloud(device))
    }

    fn operation_admission<'a>(&'a self, device: &'a DeviceRecord) -> LocalBoxFuture<'a, Result<Admission, StorageError>> {
        Box::pin(self.0.cloud_shard().operation_admission(device))
    }

    fn machine_access(&self) -> LocalBoxFuture<'_, Result<MachineAccess, CloudError>> {
        Box::pin(self.0.cloud_shard().machine_access())
    }

    fn claude_code_access(&self) -> LocalBoxFuture<'_, Result<MachineAccess, CloudError>> {
        Box::pin(self.0.cloud_shard().claude_code_access())
    }

    fn stop(&self) {
        side(&self.0).cloud.stop();
    }

    fn close(&self) -> LocalBoxFuture<'_, ()> {
        Box::pin(self.0.cloud_shard().close_cloud())
    }
}
