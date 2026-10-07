//! The `browser` kind's tabs on the backend (`live-view.md` § A browser tab
//! in the panel): a browser tab opened for each panel tab its user creates,
//! closed with the panel tab, the agent's tabs added and gone ones marked.
//! One conversation's work runs one piece at a time, so a change is never
//! based on what was read before an earlier change of the same
//! conversation.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use demi_command_package_browser_protocol::browser::{BrowserCreatedBy, BrowserTab};
use demi_plugin_interface::{PluginError, PluginPort, PortFailure, PortRefusal};
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::panel::{CreatePanelTab, PanelTab};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::sync::Mutex;

use crate::page;

/// The kind whose tabs the plugin takes part in.
pub const KIND: &str = "browser";

/// What a `browser` tab keeps.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TabData {
    url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tab: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    closed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    failure: Option<Value>,
    /// How many times the agent showed its browser tab, as the plugin last
    /// wrote it; absent while it never did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    shows: Option<u64>,
}

impl TabData {
    /// The data of `tab`; none for one that does not fit the kind, which
    /// the plugin leaves as it is.
    fn of(tab: &PanelTab) -> Option<Self> {
        serde_json::from_value(Value::Object(tab.data.clone())).ok()
    }

    /// The browser tab it shows, while the browser has it.
    fn live(&self) -> Option<&str> {
        match self.closed {
            Some(true) => None,
            _ => self.tab.as_deref(),
        }
    }
}

/// The id of the panel tab for the browser's tab `tab` that the plugin
/// adds: a tab the user closed is never added again, since an id is used
/// once.
fn added_id(tab: &str) -> String {
    format!("browser-{tab}")
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

    /// Opens a browser tab for the panel tab `id`, unless it shows one: the
    /// tab its user created, or one whose browser tab closed or failed to
    /// open, which Reload and Retry open again.
    pub async fn bind(
        &self,
        conversation: &ConversationId,
        port: &PluginPort,
        id: &str,
    ) -> Result<(), PluginError> {
        let lock = self.lock(conversation);
        let _turn = lock.lock().await;
        let Some(data) = data_of(port, id).await? else {
            return Ok(());
        };
        if data.live().is_some() && data.failure.is_none() {
            return Ok(());
        }
        let asked = data.url.clone();
        let opened = match page::open(port, &asked).await {
            Ok(opened) => opened,
            Err(error) => {
                port.update_panel_tab(id, fields([("failure", failure(&error))]))
                    .await?;
                return Ok(());
            }
        };
        let tab = opened.id.to_string();
        let bound = fields([
            ("tab", Value::String(tab.clone())),
            ("closed", Value::Null),
            ("failure", Value::Null),
        ]);
        port.update_panel_tab(id, bound).await?;
        match data_of(port, id).await? {
            // Its user closed it while it opened.
            None => page::close(port, &tab).await,
            // Its user asked for another address while it opened: the last
            // one they asked for is where it goes.
            Some(now) if now.url != asked => page::navigate(port, &tab, now.url).await.map(|_| ()),
            Some(_) => Ok(()),
        }
    }

    /// Closes the browser's tab of a panel tab its user removed.
    pub async fn removed(
        &self,
        conversation: &ConversationId,
        port: &PluginPort,
        tab: &PanelTab,
    ) -> Result<(), PluginError> {
        let lock = self.lock(conversation);
        let _turn = lock.lock().await;
        match TabData::of(tab).as_ref().and_then(TabData::live) {
            Some(browser_tab) => page::close(port, browser_tab).await,
            None => Ok(()),
        }
    }

    /// Reads the browser's tabs and updates the panel from them: a tab for
    /// each tab the agent or a page opened, each bound tab's count of the
    /// times the agent showed its browser tab when the count rose
    /// (`live-view.md` § Showing a tab), and every bound tab whose browser
    /// tab is gone marked closed.
    pub async fn sync(
        &self,
        conversation: &ConversationId,
        port: &PluginPort,
    ) -> Result<(), PluginError> {
        let lock = self.lock(conversation);
        let _turn = lock.lock().await;
        let listed = page::list(port).await?;
        let panel = port.panel_tabs().await?;
        let bound: Vec<(String, TabData)> = panel
            .tabs
            .iter()
            .filter(|tab| tab.kind == KIND)
            .filter_map(|tab| TabData::of(tab).map(|data| (tab.id.clone(), data)))
            .collect();
        let shown: HashSet<&str> = bound
            .iter()
            .filter_map(|(_, data)| data.tab.as_deref())
            .collect();
        for tab in &listed {
            if tab.created_by == (BrowserCreatedBy::User {}) || shown.contains(tab.id.as_str()) {
                continue;
            }
            let create = CreatePanelTab {
                id: added_id(tab.id.as_str()),
                kind: KIND.into(),
                data: added(tab),
                index: None,
            };
            port.create_panel_tab(create).await?;
        }
        let present: HashMap<&str, &BrowserTab> =
            listed.iter().map(|tab| (tab.id.as_str(), tab)).collect();
        for (id, data) in &bound {
            let Some(browser_tab) = data.live() else {
                continue;
            };
            match present.get(browser_tab) {
                None => {
                    port.update_panel_tab(id.as_str(), fields([("closed", Value::Bool(true))]))
                        .await?;
                }
                Some(tab) if tab.shows > data.shows.unwrap_or(0) => {
                    port.update_panel_tab(id.as_str(), fields([("shows", Value::from(tab.shows))]))
                        .await?;
                }
                Some(_) => {}
            }
        }
        Ok(())
    }
}

/// Why a browser tab could not be opened, as the tab's content shows it:
/// the browser's or the Host's own code and message.
fn failure(error: &PluginError) -> Value {
    let (code, message) = match error {
        PluginError::Refused { reason, message } => (json!(reason), message.clone()),
        PluginError::Port {
            refusal: PortRefusal::Host { code, message, .. },
        } => (json!(code), message.clone()),
        error => (json!("failed"), error.to_string()),
    };
    json!({ "code": code, "message": message })
}

/// The data of the panel tab `id`; none once the panel no longer has it.
async fn data_of(port: &PluginPort, id: &str) -> Result<Option<TabData>, PortFailure> {
    let panel = port.panel_tabs().await?;
    Ok(panel
        .tabs
        .iter()
        .find(|tab| tab.id == id)
        .and_then(TabData::of))
}

/// The data of the panel tab the plugin adds for the browser's `tab`: its
/// page's title names it in the strip until a view shows it, and the times
/// the agent showed it, once it did.
fn added(tab: &BrowserTab) -> Map<String, Value> {
    let mut data = fields([
        ("url", Value::String(tab.url.clone())),
        ("tab", Value::String(tab.id.to_string())),
        ("title", Value::String(tab.title.clone())),
    ]);
    if tab.shows > 0 {
        data.insert("shows".to_owned(), Value::from(tab.shows));
    }
    data
}

fn fields<const N: usize>(fields: [(&str, Value); N]) -> Map<String, Value> {
    fields
        .into_iter()
        .map(|(field, value)| (field.to_owned(), value))
        .collect()
}
