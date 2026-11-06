//! What whoever runs a core gives it: a server for each of its users' cores,
//! a program for its one core.
//!
//! The core decides; the starter delivers what only it can: a Cloud it wakes,
//! the pairing of a runner, the browsers a notification goes to, and the
//! records of its own that name what the core holds, such as which core a
//! runner's token or a schedule's URL reaches.

use std::rc::Rc;

use demi_core_host_access::cloud::CloudHost;
use demi_core_store::StorageError;
use demi_core_store::devices::DeviceRecord;
use demi_provider_registry::entry::ProviderEntry;
use demi_runner_client::HostLog;
use demi_shared_types::ScheduleId;
use demi_web_api_protocol::auth::UserDto;
use demi_web_api_protocol::devices::{DeviceDto, InstallCommands};
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use demi_web_api_protocol::push::PushMessage;
use demi_web_api_protocol::conversations::ConversationShares;
use futures_util::future::LocalBoxFuture;

/// What a notification did as it was sent: counted then, delivered after.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notified {
    /// It goes to this many of the user's browsers.
    Sending(usize),
    /// Every browser that would show it shows its conversation already.
    Shown,
    /// No browser of the user takes it.
    Nowhere,
}

/// How a notification reaches the user now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Reach {
    /// The user's browsers with notifications on.
    pub browsers: usize,
    /// Those of them that take the agent's notifications.
    pub agent_notifies: usize,
}

/// What the core asks of whoever runs it.
pub trait Starter {
    /// The Cloud, a Host the starter wakes and stops; none for a starter
    /// that runs no Cloud.
    fn cloud(&self) -> Option<Rc<dyn CloudHost>>;

    /// Whether the Cloud `device` reaches only the network the server's
    /// policy allows.
    fn cloud_network_limited(&self, device: &DeviceRecord) -> bool;

    /// Pairs the runner waiting under the pairing code `code`; a refusal says
    /// why in the words the agent reads.
    fn pair<'a>(&'a self, code: &'a str) -> LocalBoxFuture<'a, Result<DeviceDto, String>>;

    /// The commands that install a runner that connects to the core; none
    /// while nothing serves the installers.
    fn install_commands(&self) -> Option<InstallCommands>;

    /// Where the runner lines of `host` are kept; none to keep none.
    fn host_log(&self, host: &DeviceId) -> Option<Rc<dyn HostLog>>;

    /// The device `device` was removed: its token reaches nothing now.
    fn device_removed<'a>(&'a self, device: &'a DeviceId) -> LocalBoxFuture<'a, Result<(), StorageError>>;

    /// The schedule `id` is called at its URL: the call reaches this core.
    /// Said before the schedule is kept.
    fn schedule_called_at_url<'a>(&'a self, id: &'a ScheduleId) -> LocalBoxFuture<'a, Result<(), StorageError>>;

    /// The schedule `id` was deleted.
    fn schedule_deleted<'a>(&'a self, id: &'a ScheduleId) -> LocalBoxFuture<'a, Result<(), StorageError>>;

    /// The conversation `id` was deleted: what the starter keeps of it goes.
    fn conversation_deleted<'a>(&'a self, id: &'a ConversationId) -> LocalBoxFuture<'a, Result<(), StorageError>>;

    /// The provider entry `entry` was deleted.
    fn provider_deleted<'a>(&'a self, entry: &'a ProviderEntry) -> LocalBoxFuture<'a, Result<(), StorageError>>;

    /// Sends `message` to the user, after the notifications before it.
    fn notify(&self, message: PushMessage) -> LocalBoxFuture<'_, Result<Notified, StorageError>>;

    /// How a notification reaches the user now.
    fn reach(&self) -> LocalBoxFuture<'_, Result<Reach, StorageError>>;

    /// The user the core serves, with the role that decides which provider
    /// entries they configure.
    fn principal(&self) -> LocalBoxFuture<'_, Result<UserDto, StorageError>>;

    /// Whether the agent's notifications reach the user now.
    fn agent_reaches(&self) -> LocalBoxFuture<'_, Result<bool, StorageError>>;

    /// The conversation `id`'s shares, its own and its answers', which its
    /// page shows; none for a starter that shares nothing.
    fn conversation_shares<'a>(&'a self, id: &'a ConversationId) -> LocalBoxFuture<'a, Result<ConversationShares, StorageError>>;

    /// Ends the starter's own work on the core, first as the core closes,
    /// so nothing it serves reads what the core's close changes.
    fn closing(&self) -> LocalBoxFuture<'_, ()>;
}
