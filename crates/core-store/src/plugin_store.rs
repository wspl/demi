//! The core store as the plugin host's store
//! (`crates/plugin-host/src/store.rs`): each operation is the store's own.

use std::collections::BTreeMap;

use demi_plugin_host::store::{
    OAuthClient, PanelOutcome, PluginReport, PluginStore, PluginValue, SealedSecret, SecretRow, StoreError, Stored,
    ValueWrite, Written,
};
use demi_plugin_interface::{HostDirectory, WorkspaceName};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, WorkspaceId};
use demi_web_api_protocol::panel::{PanelChange, WorkPanel};
use futures_util::FutureExt;
use serde_json::Value;

use crate::store::CoreStore;

impl PluginStore for CoreStore {
    fn switches(&self) -> Stored<'_, BTreeMap<String, bool>> {
        async move { self.user_plugins().await.map_err(StoreError::new) }.boxed_local()
    }

    fn set_switch(&self, plugin: String, enabled: bool) -> Stored<'_, ()> {
        async move { self.set_user_plugin(plugin, enabled).await.map_err(StoreError::new) }.boxed_local()
    }

    fn feature_switches(&self) -> Stored<'_, BTreeMap<(String, String), bool>> {
        async move { self.user_plugin_features().await.map_err(StoreError::new) }.boxed_local()
    }

    fn set_feature_switch(&self, plugin: String, feature: String, enabled: bool) -> Stored<'_, ()> {
        async move {
            self.set_user_plugin_feature(plugin, feature, enabled)
                .await
                .map_err(StoreError::new)
        }
        .boxed_local()
    }

    fn value(&self, plugin: String, key: String) -> Stored<'_, Option<PluginValue>> {
        async move { self.plugin_value(plugin, key).await.map_err(StoreError::new) }.boxed_local()
    }

    fn values(&self, plugin: String) -> Stored<'_, BTreeMap<String, PluginValue>> {
        async move { self.plugin_values(plugin).await.map_err(StoreError::new) }.boxed_local()
    }

    fn write_value(&self, write: ValueWrite) -> Stored<'_, Written> {
        async move { self.write_plugin_value(write).await.map_err(StoreError::new) }.boxed_local()
    }

    fn remove_value(&self, plugin: String, key: String, revision: u64) -> Stored<'_, Written> {
        async move {
            self.remove_plugin_value(plugin, key, revision)
                .await
                .map_err(StoreError::new)
        }
        .boxed_local()
    }

    fn directories(&self) -> Stored<'_, BTreeMap<String, Vec<HostDirectory>>> {
        async move { self.plugin_directories().await.map_err(StoreError::new) }.boxed_local()
    }

    fn set_directories(&self, plugin: String, directories: Vec<HostDirectory>) -> Stored<'_, ()> {
        async move {
            self.set_plugin_directories(plugin, directories)
                .await
                .map_err(StoreError::new)
        }
        .boxed_local()
    }

    fn secret(&self, row: SecretRow) -> Stored<'_, Option<SealedSecret>> {
        async move { self.plugin_secret(row).await.map_err(StoreError::new) }.boxed_local()
    }

    fn write_secret(&self, row: SecretRow, sealed: Vec<u8>, revision: Option<u64>) -> Stored<'_, Written> {
        async move {
            self.write_plugin_secret(row, sealed, revision)
                .await
                .map_err(StoreError::new)
        }
        .boxed_local()
    }

    fn remove_secret(&self, row: SecretRow, revision: u64) -> Stored<'_, Written> {
        async move { self.remove_plugin_secret(row, revision).await.map_err(StoreError::new) }.boxed_local()
    }

    fn oauth_client(&self, plugin: String, issuer: String) -> Stored<'_, Option<OAuthClient>> {
        async move { self.plugin_oauth_client(plugin, issuer).await.map_err(StoreError::new) }.boxed_local()
    }

    fn set_oauth_client(&self, plugin: String, issuer: String, client: OAuthClient) -> Stored<'_, ()> {
        async move {
            self.set_plugin_oauth_client(plugin, issuer, client)
                .await
                .map_err(StoreError::new)
        }
        .boxed_local()
    }

    fn forget_oauth_client(&self, plugin: String, issuer: String, client_id: String) -> Stored<'_, ()> {
        async move {
            self.forget_plugin_oauth_client(plugin, issuer, client_id)
                .await
                .map_err(StoreError::new)
        }
        .boxed_local()
    }

    fn report(&self, plugin: String, conversation: ConversationId, device: DeviceId) -> Stored<'_, Option<PluginReport>> {
        async move {
            self.plugin_report(plugin, conversation, device)
                .await
                .map_err(StoreError::new)
        }
        .boxed_local()
    }

    fn keep_report(
        &self,
        plugin: String,
        conversation: ConversationId,
        device: DeviceId,
        revision: u64,
        value: Value,
    ) -> Stored<'_, bool> {
        async move {
            self.keep_plugin_report(plugin, conversation, device, revision, value)
                .await
                .map_err(StoreError::new)
        }
        .boxed_local()
    }

    fn panel(&self, conversation: ConversationId) -> Stored<'_, WorkPanel> {
        async move { CoreStore::panel(self, conversation).await.map_err(StoreError::new) }.boxed_local()
    }

    fn change_panel(&self, conversation: ConversationId, changes: Vec<PanelChange>) -> Stored<'_, PanelOutcome> {
        async move {
            CoreStore::change_panel(self, conversation, changes)
                .await
                .map_err(StoreError::new)
        }
        .boxed_local()
    }

    fn close_panel_kinds(&self, kinds: Vec<String>) -> Stored<'_, Vec<ConversationId>> {
        async move {
            CoreStore::close_panel_kinds(self, kinds)
                .await
                .map_err(StoreError::new)
        }
        .boxed_local()
    }

    fn workspaces(&self) -> Stored<'_, Vec<WorkspaceName>> {
        async move {
            let records = CoreStore::workspaces(self).await.map_err(StoreError::new)?;
            Ok(records
                .into_iter()
                .map(|record| WorkspaceName {
                    name: record.shown_name().to_owned(),
                    id: record.id,
                })
                .collect())
        }
        .boxed_local()
    }

    fn conversation_workspace(&self, conversation: ConversationId) -> Stored<'_, Option<WorkspaceId>> {
        async move {
            let record = self.conversation(conversation).await.map_err(StoreError::new)?;
            Ok(record.map(|record| record.workspace))
        }
        .boxed_local()
    }

    fn primary_device(&self, conversation: ConversationId) -> Stored<'_, Option<DeviceId>> {
        async move {
            let Some(record) = self.conversation(conversation).await.map_err(StoreError::new)? else {
                return Ok(None);
            };
            let target = self
                .execution_target(record.id, record.target)
                .await
                .map_err(StoreError::new)?;
            Ok(target.device().cloned())
        }
        .boxed_local()
    }
}
