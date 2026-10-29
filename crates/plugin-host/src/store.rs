//! What the plugin host keeps beyond the process, which the program that runs
//! it stores, and the changes it tells the program's pages of.
//!
//! The host reaches both through [`PluginStore`] and [`PluginChanges`] alone:
//! each user's switches and Host directories, the plugins' values, secrets,
//! reports and OAuth clients, each conversation's work panel and the Host it
//! works on.

use std::collections::BTreeMap;

use demi_plugin_interface::{HostDirectory, WorkspaceName};
use demi_shared_types::{BlobRef, Timestamp};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId, WorkspaceId};
use demi_web_api_protocol::panel::{PanelChange, PanelEffect, WorkPanel};
use futures_util::future::LocalBoxFuture;
use serde_json::Value;

/// A failure of the program's storage, which the host passes on.
#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct StoreError(Box<dyn std::error::Error + Send + Sync>);

impl StoreError {
    pub fn new(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self(Box::new(error))
    }

    /// A stored value outside its type, which nothing repairs.
    pub fn corrupt(reason: String) -> Self {
        Self(reason.into())
    }

    /// The store's own error, when it is an `E`.
    pub fn downcast<E: std::error::Error + 'static>(self) -> Result<E, Self> {
        self.0.downcast::<E>().map(|error| *error).map_err(Self)
    }
}

/// What a store's operation answers.
pub type Stored<'a, T> = LocalBoxFuture<'a, Result<T, StoreError>>;

/// A stored value and its revision.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginValue {
    pub document: Value,
    pub revision: u64,
}

/// A write of a plugin value.
pub struct ValueWrite {
    pub user: UserId,
    pub plugin: String,
    pub key: String,
    pub document: Value,
    /// The revision the write read; none for a value that does not exist
    /// yet.
    pub revision: Option<u64>,
    /// The blobs the value names.
    pub blobs: Vec<BlobRef>,
}

/// What a conditional write of a plugin value or secret did.
#[derive(Debug)]
pub enum Written {
    /// The value's new revision.
    Revision(u64),
    /// Another write came first.
    Conflict,
}

/// A sealed secret and its revision.
#[derive(Debug, Clone, PartialEq)]
pub struct SealedSecret {
    pub sealed: Vec<u8>,
    pub revision: u64,
}

/// Where a secret is: its plugin, its user and its name, which its sealing
/// binds it to.
#[derive(Debug, Clone)]
pub struct SecretRow {
    pub user: UserId,
    pub plugin: String,
    pub name: String,
}

/// A client a plugin registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthClient {
    pub redirect_uri: String,
    pub client_id: String,
}

/// A report as last kept.
#[derive(Debug, Clone, PartialEq)]
pub struct PluginReport {
    /// The package's own count of its changes.
    pub revision: u64,
    pub value: Value,
    pub reported_at: Timestamp,
}

/// What a request of changes came to.
#[derive(Debug, Clone, PartialEq)]
pub enum PanelOutcome {
    /// The changes are in the panel, which is at `revision` now; each that
    /// had something to do did this.
    Changed {
        revision: u64,
        effects: Vec<PanelEffect>,
    },
    /// The changes had nothing to do; the panel stays at `revision`.
    Unchanged {
        revision: u64,
    },
    /// A create past the most tabs or the most data.
    Full,
    /// An update past the most data.
    TooLarge,
    /// No conversation has that id.
    Missing,
}

/// The storage of the plugin host, which the program implements.
pub trait PluginStore {
    /// Whether the user turned each plugin on, by its id; a plugin the user
    /// never switched is on.
    fn switches(&self, user: UserId) -> Stored<'_, BTreeMap<String, bool>>;

    fn set_switch(&self, user: UserId, plugin: String, enabled: bool) -> Stored<'_, ()>;

    fn value(&self, user: UserId, plugin: String, key: String) -> Stored<'_, Option<PluginValue>>;

    fn values(&self, user: UserId, plugin: String) -> Stored<'_, BTreeMap<String, PluginValue>>;

    /// Writes the value if it is still at the revision the write read.
    fn write_value(&self, write: ValueWrite) -> Stored<'_, Written>;

    /// Removes the value if it is still at `revision`.
    fn remove_value(&self, user: UserId, plugin: String, key: String, revision: u64) -> Stored<'_, Written>;

    /// Each plugin's Host directories for the user, by the plugin's id.
    fn directories(&self, user: UserId) -> Stored<'_, BTreeMap<String, Vec<HostDirectory>>>;

    fn set_directories(&self, user: UserId, plugin: String, directories: Vec<HostDirectory>) -> Stored<'_, ()>;

    fn secret(&self, row: SecretRow) -> Stored<'_, Option<SealedSecret>>;

    /// Writes the sealed secret if it is still at `revision`; none for a
    /// secret that does not exist yet.
    fn write_secret(&self, row: SecretRow, sealed: Vec<u8>, revision: Option<u64>) -> Stored<'_, Written>;

    /// Removes the secret if it is still at `revision`.
    fn remove_secret(&self, row: SecretRow, revision: u64) -> Stored<'_, Written>;

    /// The client the plugin registered with the authorization server
    /// `issuer`.
    fn oauth_client(&self, plugin: String, issuer: String) -> Stored<'_, Option<OAuthClient>>;

    fn set_oauth_client(&self, plugin: String, issuer: String, client: OAuthClient) -> Stored<'_, ()>;

    /// Forgets the client `client_id`, which the vendor no longer knows; a
    /// client registered since stays.
    fn forget_oauth_client(&self, plugin: String, issuer: String, client_id: String) -> Stored<'_, ()>;

    /// What the plugin's package on the Host of `device` last reported of
    /// the conversation; none before it reported.
    fn report(&self, plugin: String, conversation: ConversationId, device: DeviceId) -> Stored<'_, Option<PluginReport>>;

    /// Keeps `value` as what the plugin's package on the Host of `device`
    /// reported of the conversation now; answers whether it differs from
    /// what was kept. A conversation that is gone keeps nothing.
    fn keep_report(
        &self,
        plugin: String,
        conversation: ConversationId,
        device: DeviceId,
        revision: u64,
        value: Value,
    ) -> Stored<'_, bool>;

    /// The conversation's work panel.
    fn panel(&self, conversation: ConversationId) -> Stored<'_, WorkPanel>;

    fn change_panel(&self, conversation: ConversationId, changes: Vec<PanelChange>) -> Stored<'_, PanelOutcome>;

    /// Removes the tabs of `kinds` from every panel of `owner`'s
    /// conversations, and answers the conversations whose panel changed.
    fn close_panel_kinds(&self, owner: UserId, kinds: Vec<String>) -> Stored<'_, Vec<ConversationId>>;

    /// The device the conversation works on, from the records: none for a
    /// conversation whose Host does not exist yet. It asks no Host.
    fn primary_device(&self, conversation: ConversationId) -> Stored<'_, Option<DeviceId>>;

    /// The user's workspaces in the user's order, each with its name as
    /// Demi's commands say it.
    fn workspaces(&self, user: UserId) -> Stored<'_, Vec<WorkspaceName>>;

    /// The workspace the conversation is in; none when no conversation has
    /// the id.
    fn conversation_workspace(&self, conversation: ConversationId) -> Stored<'_, Option<WorkspaceId>>;
}

/// The changes the host tells one user's pages of.
pub trait PluginChanges {
    /// The list of plugins, with whether each is on.
    fn plugins_changed(&self);

    /// The plugin's state for the user.
    fn plugin_changed(&self, plugin: String);

    /// The conversation's entry in the list, which carries its work panel.
    fn conversation_changed(&self, conversation: ConversationId);

    /// The conversation's metadata, which carries each plugin's revision of
    /// its state in the conversation.
    fn metadata_changed(&self, conversation: ConversationId);
}
