//! The program that opened a core, as the core's routes see their caller
//! and as the core sees its starter.

use std::rc::Rc;
use std::sync::Arc;

use demi_core::Core;
use demi_core::starter::{Notified, Reach, Starter};
use demi_core_host_access::cloud::CloudHost;
use demi_core_store::StorageError;
use demi_core_store::devices::DeviceRecord;
use demi_core_sync::Part;
use demi_provider_registry::entry::ProviderEntry;
use demi_runner_client::HostLog;
use demi_shared_types::ScheduleId;
use demi_web_api_protocol::connection::LaneTarget;
use demi_web_api_protocol::conversations::ConversationShares;
use demi_web_api_protocol::devices::{DeviceDto, InstallCommands};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId};
use demi_web_api_protocol::push::PushMessage;
use demi_web_api_protocol::starter::NotifyOutcome;
use demi_web_api_protocol::state::SyncEvent;
use futures_util::future::{BoxFuture, LocalBoxFuture};
use url::Url;

use super::cores::{OpenCores, Serving, not_open};
use crate::caller::{Caller, CallerPage, Gone};
use crate::error::ApiError;
use crate::starter_lane::StarterLane;
use crate::state::StateError;

/// The program, calling the serving core `principal`.
#[derive(Clone)]
pub(crate) struct ProgramCaller {
    principal: UserId,
    /// The core's id as text: the File view's grants of the program's
    /// calls are kept under it.
    key: String,
    core: Serving,
    cores: Arc<OpenCores>,
}

impl ProgramCaller {
    pub(crate) fn new(principal: UserId, core: Serving, cores: Arc<OpenCores>) -> Self {
        Self {
            key: principal.to_string(),
            principal,
            core,
            cores,
        }
    }
}

impl Caller for ProgramCaller {
    fn principal(&self) -> &UserId {
        &self.principal
    }

    fn key(&self) -> &str {
        &self.key
    }

    fn present(&self) -> BoxFuture<'_, Result<bool, ApiError>> {
        Box::pin(async move { Ok(self.cores.serving(&self.principal).is_some()) })
    }

    /// The program proves nothing: it holds the core.
    fn proved_lately(&self) -> Result<(), ApiError> {
        Ok(())
    }

    fn opens(&self, _: &LaneTarget) -> BoxFuture<'_, Result<Arc<dyn Caller>, ApiError>> {
        Box::pin(async move {
            let core = self.cores.serving(&self.principal).ok_or_else(not_open)?;
            let now: Arc<dyn Caller> = Arc::new(Self::new(self.principal.clone(), core, self.cores.clone()));
            Ok(now)
        })
    }

    fn gone(&self) -> BoxFuture<'static, Gone> {
        let closed = self.core.closed.clone();
        Box::pin(async move {
            closed.cancelled().await;
            Gone::Ended
        })
    }

    fn page(&self, _: &Core) -> Box<dyn CallerPage> {
        Box::new(ProgramPage)
    }

    fn starter_lane(&self) -> Option<Arc<StarterLane>> {
        Some(self.core.starter_lane.clone())
    }
}

/// The program's page: it follows the core's parts alone, and what it shows
/// decides nothing, since the core's notifications go on its starter lane.
struct ProgramPage;

impl CallerPage for ProgramPage {
    fn only(&self) -> Option<Part> {
        None
    }

    fn read<'a>(&'a self, _: &'a Core, _: &'a Part) -> LocalBoxFuture<'a, Result<Option<SyncEvent>, StateError>> {
        Box::pin(async { Ok(None) })
    }

    fn sent(&mut self, _: &SyncEvent) -> Vec<Part> {
        Vec::new()
    }

    fn show(&self, _: Option<ConversationId>, _: bool) {}

    fn used(&self) {}
}

/// What the program gives one of its cores: its notifications go on the
/// core's starter lane; a program's core runs no Cloud, pairs no runner and
/// keeps no record of its own of what the core holds.
pub(crate) struct ProgramStarter {
    starter_lane: Arc<StarterLane>,
    /// Where the program's server forwards the core's routes, under `/api`.
    public_url: Option<Url>,
}

impl ProgramStarter {
    pub(crate) fn new(starter_lane: Arc<StarterLane>, public_url: Option<Url>) -> Self {
        Self {
            starter_lane,
            public_url,
        }
    }
}

impl Starter for ProgramStarter {
    fn cloud(&self) -> Option<Rc<dyn CloudHost>> {
        None
    }

    fn cloud_network_limited(&self, _: &DeviceRecord) -> bool {
        false
    }

    fn pair<'a>(&'a self, _: &'a str) -> LocalBoxFuture<'a, Result<DeviceDto, String>> {
        Box::pin(async {
            Err("This core pairs no runner: its Hosts reach it over channels the program gives.".to_owned())
        })
    }

    fn install_commands(&self) -> Option<InstallCommands> {
        None
    }

    fn public_url(&self) -> Option<Url> {
        self.public_url.clone()
    }

    fn host_log(&self, _: &DeviceId) -> Option<Rc<dyn HostLog>> {
        None
    }

    fn device_removed<'a>(&'a self, _: &'a DeviceId) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        Box::pin(async { Ok(()) })
    }

    /// The process finds the schedule among its cores when its URL is called.
    fn schedule_called_at_url<'a>(&'a self, _: &'a ScheduleId) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        Box::pin(async { Ok(()) })
    }

    fn schedule_deleted<'a>(&'a self, _: &'a ScheduleId) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        Box::pin(async { Ok(()) })
    }

    fn conversation_deleted<'a>(&'a self, _: &'a ConversationId) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        Box::pin(async { Ok(()) })
    }

    fn provider_deleted<'a>(&'a self, _: &'a ProviderEntry) -> LocalBoxFuture<'a, Result<(), StorageError>> {
        Box::pin(async { Ok(()) })
    }

    fn notify(&self, message: PushMessage) -> LocalBoxFuture<'_, Result<Notified, StorageError>> {
        Box::pin(async move {
            Ok(match self.starter_lane.notify(message).await {
                NotifyOutcome::Sending { to } => Notified::Sending(to),
                NotifyOutcome::Shown => Notified::Shown,
                NotifyOutcome::Nowhere => Notified::Nowhere,
            })
        })
    }

    /// The program is one receiver while its starter lane is open.
    fn reach(&self) -> LocalBoxFuture<'_, Result<Reach, StorageError>> {
        let open = usize::from(self.starter_lane.is_open());
        Box::pin(async move {
            Ok(Reach {
                browsers: open,
                agent_notifies: open,
            })
        })
    }

    fn agent_reaches(&self) -> LocalBoxFuture<'_, Result<bool, StorageError>> {
        let open = self.starter_lane.is_open();
        Box::pin(async move { Ok(open) })
    }

    fn conversation_shares<'a>(&'a self, _: &'a ConversationId) -> LocalBoxFuture<'a, Result<ConversationShares, StorageError>> {
        Box::pin(async { Ok(ConversationShares { shares: Vec::new() }) })
    }

    fn closing(&self) -> LocalBoxFuture<'_, ()> {
        self.starter_lane.end();
        Box::pin(async {})
    }
}
