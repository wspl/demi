//! The `mobile` kind's tabs on the backend (`crates/plugin-mobile/plugin/src/lib.rs`):
//! one tab for each device the conversation has open on its own Host.
//!
//! - A tab is added when the agent opens the device.
//! - A tab is removed when the device closes, is lost, or goes with the
//!   package's state on the Host.
//! - Closing a tab closes its device, as `demi mobile close` does.
//! - A device the agent opens on an attached Host has no tab: the panel shows
//!   the conversation's Host's devices alone.
//!
//! One conversation's work runs one piece at a time.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use demi_plugin_interface::{PluginError, PluginPort};
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::panel::{CreatePanelTab, PanelTab};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use tokio::sync::Mutex;

use crate::page;

/// The kind whose tabs the plugin takes part in.
pub const KIND: &str = "mobile";

/// What a `mobile` tab keeps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TabData {
    /// The device's handle, `m1`.
    device: String,
    /// Its simctl id or AVD name.
    native_id: String,
    /// Its name, which names the tab.
    name: String,
}

impl TabData {
    fn of(tab: &PanelTab) -> Option<Self> {
        (tab.kind == KIND)
            .then(|| serde_json::from_value(Value::Object(tab.data.clone())).ok())
            .flatten()
    }
}

/// The id of the tab the plugin adds for the device `device`: a handle is
/// given once in a conversation, so a closed tab is never added again.
fn added_id(device: &str) -> String {
    format!("mobile-{device}")
}

/// One conversation's work at a time.
#[derive(Default)]
pub struct Work {
    locks: RefCell<HashMap<ConversationId, Rc<Mutex<()>>>>,
}

impl Work {
    fn lock(&self, conversation: &ConversationId) -> Rc<Mutex<()>> {
        self.locks
            .borrow_mut()
            .entry(conversation.clone())
            .or_default()
            .clone()
    }

    /// Closes the device of a tab its user closed.
    pub async fn removed(
        &self,
        conversation: &ConversationId,
        port: &PluginPort,
        tab: &PanelTab,
    ) -> Result<(), PluginError> {
        let lock = self.lock(conversation);
        let _turn = lock.lock().await;
        match TabData::of(tab) {
            Some(data) => page::close(port, &data.device).await,
            None => Ok(()),
        }
    }

    /// What the package kept on a Host ended: the panel shows what the
    /// conversation's Host has open now, which loses the tabs of the
    /// devices that went with it; a stopped Host has none open.
    pub async fn lost(&self, conversation: &ConversationId, port: &PluginPort) -> Result<(), PluginError> {
        let lock = self.lock(conversation);
        let _turn = lock.lock().await;
        let open = page::list(port).await?.map(|listed| listed.devices).unwrap_or_default();
        self.show(port, &open).await
    }

    /// Reads the Host's devices and updates the panel: a tab for each device
    /// this conversation has open that has none, and each tab whose device
    /// is no longer open removed.
    pub async fn sync(
        &self,
        conversation: &ConversationId,
        port: &PluginPort,
    ) -> Result<(), PluginError> {
        let lock = self.lock(conversation);
        let _turn = lock.lock().await;
        // A stopped Host's devices are as the panel last showed them, until
        // its package's loss says otherwise.
        let Some(listed) = page::list(port).await? else {
            return Ok(());
        };
        self.show(port, &listed.devices).await
    }

    /// A tab for each of `listed` this conversation has open that has none,
    /// and each tab whose device is not among them removed.
    async fn show(
        &self,
        port: &PluginPort,
        listed: &[demi_command_package_mobile_protocol::mobile::Simulator],
    ) -> Result<(), PluginError> {
        let open: Vec<(&str, &demi_command_package_mobile_protocol::mobile::Simulator)> = listed
            .iter()
            .filter_map(|simulator| simulator.open.as_ref().map(|device| (device.as_str(), simulator)))
            .collect();
        let panel = port.panel_tabs().await?;
        let bound: Vec<(String, TabData)> = panel
            .tabs
            .iter()
            .filter_map(|tab| TabData::of(tab).map(|data| (tab.id.clone(), data)))
            .collect();
        let shown: HashSet<&str> = bound.iter().map(|(_, data)| data.device.as_str()).collect();
        for (device, simulator) in &open {
            if shown.contains(device) {
                continue;
            }
            let mut data = fields([
                ("device", Value::from(*device)),
                ("nativeId", Value::from(simulator.native_id.clone())),
                ("name", Value::from(simulator.name.clone())),
                ("platform", Value::from(simulator.platform.to_string())),
            ]);
            // The model the driver reads, which the toolbar shows after the
            // name; a device whose driver knows none has none.
            if !simulator.model.is_empty() {
                data.insert("model".into(), Value::from(simulator.model.clone()));
            }
            let create = CreatePanelTab {
                id: added_id(device),
                kind: KIND.into(),
                data,
                index: None,
            };
            port.create_panel_tab(create).await?;
        }
        let present: HashSet<&str> = open.iter().map(|(device, _)| *device).collect();
        for (id, data) in &bound {
            if !present.contains(data.device.as_str()) {
                port.remove_panel_tab(id.as_str()).await?;
            }
        }
        Ok(())
    }
}

fn fields<const N: usize>(fields: [(&str, Value); N]) -> Map<String, Value> {
    fields
        .into_iter()
        .map(|(field, value)| (field.to_owned(), value))
        .collect()
}

