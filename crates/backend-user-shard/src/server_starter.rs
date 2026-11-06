//! What the server gives each of its users' cores.

use std::rc::{Rc, Weak};
use std::sync::Arc;

use demi_core_host_access::cloud::CloudHost;
use demi_core_store::StorageError;
use demi_core_store::devices::DeviceRecord;
use demi_backend_runners::claims::ClaimRefusal;
use demi_backend_runners::install::install_commands;
use demi_provider_registry::entry::ProviderEntry;
use demi_runner_client::HostLog;
use demi_shared_types::ScheduleId;
use demi_web_api_protocol::auth::UserDto;
use demi_web_api_protocol::devices::{DeviceDto, InstallCommands};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId};
use demi_web_api_protocol::push::PushMessage;
use futures_util::future::LocalBoxFuture;

use crate::page_notifications::PageNotifications;
use crate::services::Services;
use crate::shard::Shard;
use crate::starter::{Notified, Reach, Starter};

/// The server's side of one user's core.
pub struct ServerStarter {
    user: UserId,
    services: Arc<Services>,
    /// Weak: the core owns its starter.
    shard: Weak<Shard>,
    /// The user's open pages, which decide where a notification goes.
    pub(crate) pages: PageNotifications,
}

impl ServerStarter {
    pub fn new(user: UserId, services: Arc<Services>, shard: Weak<Shard>) -> Self {
        Self {
            user,
            services,
            shard,
            pages: PageNotifications::default(),
        }
    }

    /// The user whose core it is.
    pub(crate) fn user(&self) -> &UserId {
        &self.user
    }

    /// The core.
    pub(crate) fn shard(&self) -> &Weak<Shard> {
        &self.shard
    }

    /// The server's side of `core`, which the server started.
    pub fn of(core: &Shard) -> &Self {
        core.starter()
            .as_any()
            .downcast_ref()
            .expect("the server starts each of its cores with its own side")
    }

    /// The server's services.
    pub fn services(&self) -> &Arc<Services> {
        &self.services
    }
}

impl Starter for ServerStarter {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn cloud(&self) -> Option<Rc<dyn CloudHost>> {
        Some(self.shard.upgrade()?)
    }

    fn cloud_network_limited(&self, device: &DeviceRecord) -> bool {
        demi_backend_cloud::isolation::network_policy_applies(&self.services.cloud, device.id.as_str())
    }

    fn pair<'a>(&'a self, code: &'a str) -> LocalBoxFuture<'a, Result<DeviceDto, String>> {
        Box::pin(async move {
            let shard = self.shard.upgrade().ok_or("the core is closing")?;
            match self.services.claims.claim(shard.store(), &self.user, code).await {
                Ok(device) => Ok(device),
                Err(ClaimRefusal::UnknownCode) => Err(format!(
                    "{code}: no computer waits under this pairing code; it expired or was never issued. The installer shows a new code when one expires: read the newest one it shows and pass that."
                )),
                Err(ClaimRefusal::TooManyAttempts) => Err(
                    "Too many pairing codes were tried in the last minute; wait a minute, then pass the newest code the installer shows.".into(),
                ),
                Err(ClaimRefusal::Storage(error)) => Err(error.to_string()),
            }
        })
    }

    fn install_commands(&self) -> Option<InstallCommands> {
        let backend = self.services.public_url.get()?;
        Some(install_commands(backend.url()))
    }

    fn host_log(&self, host: &DeviceId) -> Option<Rc<dyn HostLog>> {
        Some(Rc::new(HostLines {
            log: self.services.log.clone(),
            host: host.to_string(),
            user: self.user.clone(),
        }))
    }

    fn device_removed<'a>(&'a self, device: &'a DeviceId) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        Box::pin(self.services.control.forget_device_owner(device.clone()))
    }

    fn schedule_called_at_url<'a>(&'a self, id: &'a ScheduleId) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        Box::pin(self.services.control.record_schedule_owner(id.clone(), self.user.clone()))
    }

    fn schedule_deleted<'a>(&'a self, id: &'a ScheduleId) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        Box::pin(self.services.control.forget_schedule_owner(id.clone()))
    }

    fn conversation_deleted<'a>(&'a self, id: &'a ConversationId) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        Box::pin(self.services.control.remove_shares_of(self.user.clone(), id.clone()))
    }

    fn provider_deleted<'a>(&'a self, entry: &'a ProviderEntry) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        Box::pin(async move {
            let lent = self.services.lent_keys(entry).await?;
            self.services.control.delete_share_keys_of(entry.owner.clone(), entry.id.clone()).await?;
            self.services.end_lending(&lent);
            Ok(())
        })
    }

    fn notify(&self, message: PushMessage) -> LocalBoxFuture<'_, Result<Notified, StorageError>> {
        Box::pin(async move {
            let in_use = self.pages.in_use();
            let recipients = self.recipients(&message, &in_use).await?;
            let notified = match (recipients.now.len(), recipients.shown) {
                (0, 0) => Notified::Nowhere,
                (0, _) => Notified::Shown,
                (n, _) => Notified::Sending(n),
            };
            self.deliver(message, recipients);
            Ok(notified)
        })
    }

    fn reach(&self) -> LocalBoxFuture<'_, Result<Reach, StorageError>> {
        Box::pin(async move {
            let subscriptions = self.services.push.subscriptions(self.user.clone()).await?;
            Ok(Reach {
                browsers: subscriptions.len(),
                agent_notifies: subscriptions
                    .iter()
                    .filter(|subscription| subscription.events.agent_notifies)
                    .count(),
            })
        })
    }

    fn principal(&self) -> LocalBoxFuture<'_, Result<UserDto, StorageError>> {
        Box::pin(async move {
            let account = self.services.control.account(self.user.clone()).await?;
            account.map(|account| account.user).ok_or_else(|| StorageError::Corrupt {
                table: "users",
                column: "id",
                reason: format!("the account of the core's user {} is gone", self.user),
            })
        })
    }

    fn agent_reaches(&self) -> LocalBoxFuture<'_, Result<bool, StorageError>> {
        Box::pin(self.services.control.agent_reaches(self.user.clone()))
    }
}

/// One Host's runner lines, stored in the server's log under the Host's ID.
struct HostLines {
    log: demi_backend_database::server_log::ServerLog,
    host: String,
    /// The Host's owner, whose lines they are.
    user: UserId,
}

impl HostLog for HostLines {
    fn since(&self) -> LocalBoxFuture<'static, Option<u64>> {
        let log = self.log.clone();
        let host = self.host.clone();
        Box::pin(async move { log.host_since(&host).await })
    }

    fn store(&self, lines: Vec<demi_runner_protocol::wire::HostLogLine>, missed: u64) {
        self.log.store_host(&self.host, self.user.as_str(), lines, missed);
    }
}
