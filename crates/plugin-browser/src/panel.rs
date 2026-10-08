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
    /// The panel tab whose page opened it: by Open Link in New Tab, which
    /// the page writes, or by a link or script of the page, which the plugin
    /// writes when it adds the tab.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    opened_by: Option<String>,
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
    /// tab its user created, one that failed to open, which Retry opens
    /// again, and one whose browser tab the browser lost, shown again.
    /// Answers the browser tab the panel tab shows now; none when the panel
    /// no longer has it or it could not open one, which its data says: any
    /// step that fails is the tab's `failure`, which its content shows with
    /// Retry (`live-view.md` § A browser tab in the panel), since nobody waits
    /// for the answer to the change that created the tab.
    pub async fn bind(
        &self,
        conversation: &ConversationId,
        port: &PluginPort,
        id: &str,
    ) -> Result<Option<String>, PluginError> {
        let lock = self.lock(conversation);
        let _turn = lock.lock().await;
        let Some(data) = data_of(port, id).await? else {
            return Ok(None);
        };
        if let Some(tab) = data.live()
            && data.failure.is_none()
        {
            return Ok(Some(tab.to_owned()));
        }
        match open_for(port, id, data).await {
            Ok(tab) => Ok(tab),
            Err(error) => {
                port.update_panel_tab(id, fields([("failure", failure(&error))]))
                    .await?;
                Ok(None)
            }
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
    /// each tab the agent or a page opened, a page's beside its opener's
    /// (`after_opened`) and the agent's after the others, each bound tab's
    /// count of the times the agent showed its browser tab when the count rose
    /// (`live-view.md` § Showing a tab), every bound tab whose browser tab
    /// was closed on purpose removed, and every one whose browser tab was
    /// lost with the browser marked closed, to open again when shown
    /// (`live-view.md` § A browser tab in the panel).
    pub async fn sync(
        &self,
        conversation: &ConversationId,
        port: &PluginPort,
    ) -> Result<(), PluginError> {
        let lock = self.lock(conversation);
        let _turn = lock.lock().await;
        let page::Listing {
            tabs: listed,
            closed,
        } = page::list(port).await?;
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
        // The panel's tabs in order, as this sync leaves them.
        let mut order: Vec<Placed> = panel.tabs.iter().map(Placed::of).collect();
        for tab in &listed {
            if tab.created_by == (BrowserCreatedBy::User {}) || shown.contains(tab.id.as_str()) {
                continue;
            }
            let opener = opener_of(&order, tab);
            let index = opener.map(|at| after_opened(&order, at));
            let opened_by = opener.map(|at| order[at].id.clone());
            let id = added_id(tab.id.as_str());
            let create = CreatePanelTab {
                id: id.clone(),
                kind: KIND.into(),
                data: added(tab, opened_by.as_deref()),
                index,
            };
            port.create_panel_tab(create).await?;
            let placed = Placed {
                id,
                shows: Some(tab.id.to_string()),
                opened_by,
            };
            order.insert(index.unwrap_or(order.len()), placed);
        }
        let present: HashMap<&str, &BrowserTab> =
            listed.iter().map(|tab| (tab.id.as_str(), tab)).collect();
        for (id, data) in &bound {
            let Some(browser_tab) = data.live() else {
                continue;
            };
            match present.get(browser_tab) {
                None if closed.iter().any(|tab| tab.as_str() == browser_tab) => {
                    port.remove_panel_tab(id.as_str()).await?;
                }
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

/// A panel tab as the sync places others beside it: the browser tab it
/// shows, while the browser has it, and the panel tab that opened it.
struct Placed {
    id: String,
    shows: Option<String>,
    opened_by: Option<String>,
}

impl Placed {
    fn of(tab: &PanelTab) -> Self {
        let data = (tab.kind == KIND).then(|| TabData::of(tab)).flatten();
        Self {
            id: tab.id.clone(),
            shows: data.as_ref().and_then(|data| data.live().map(str::to_owned)),
            opened_by: data.and_then(|data| data.opened_by),
        }
    }
}

/// Where in `order` the panel tab stands that shows the browser tab whose
/// page opened `tab`; none for a tab no page opened, or whose opener the
/// panel does not show.
fn opener_of(order: &[Placed], tab: &BrowserTab) -> Option<usize> {
    let BrowserCreatedBy::Page { opener } = &tab.created_by else {
        return None;
    };
    order
        .iter()
        .position(|placed| placed.shows.as_deref() == Some(opener.as_str()))
}

/// Where a tab the panel tab at `at` opened goes: right after it, behind the
/// tabs it opened before that still stand right after it, as Chrome places
/// the tabs a link opens (`live-view.md` § A browser tab in the panel). The
/// page places Open Link in New Tab by the same rule, from the same
/// `openedBy`.
fn after_opened(order: &[Placed], at: usize) -> usize {
    let opener = order[at].id.as_str();
    let run = order[at + 1..]
        .iter()
        .take_while(|placed| placed.opened_by.as_deref() == Some(opener))
        .count();
    at + 1 + run
}

/// Gives the panel tab `id`, with `data`, a browser tab on the address its
/// user asked for last. A tab whose browser tab opened but whose address could
/// not load there loads it in that tab again, as a browser's Reload does after
/// a load that failed; any other opens one on its address. If the user asked
/// for another address meanwhile, the tab loads that one; if the user closed
/// the panel tab meanwhile, its browser tab closes.
async fn open_for(
    port: &PluginPort,
    id: &str,
    data: TabData,
) -> Result<Option<String>, PluginError> {
    let (tab, at) = match data.live() {
        Some(tab) => (tab.to_owned(), None),
        None => {
            let opened = page::open(port, &data.url).await?;
            (opened.id.to_string(), Some(data.url))
        }
    };
    let bound = fields([
        ("tab", Value::String(tab.clone())),
        ("closed", Value::Null),
        ("failure", Value::Null),
    ]);
    port.update_panel_tab(id, bound).await?;
    match data_of(port, id).await? {
        None => page::close(port, &tab).await.map(|()| None),
        Some(now) if at.as_ref() != Some(&now.url) => page::navigate(port, &tab, now.url)
            .await
            .map(|_| Some(tab)),
        Some(_) => Ok(Some(tab)),
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
/// page's title names it in the strip until a view shows it, the times the
/// agent showed it, once it did, and the panel tab `opener` whose page
/// opened it.
fn added(tab: &BrowserTab, opener: Option<&str>) -> Map<String, Value> {
    let mut data = fields([
        ("url", Value::String(tab.url.clone())),
        ("tab", Value::String(tab.id.to_string())),
        ("title", Value::String(tab.title.clone())),
    ]);
    if tab.shows > 0 {
        data.insert("shows".to_owned(), Value::from(tab.shows));
    }
    if let Some(opener) = opener {
        data.insert("openedBy".to_owned(), Value::String(opener.to_owned()));
    }
    data
}

fn fields<const N: usize>(fields: [(&str, Value); N]) -> Map<String, Value> {
    fields
        .into_iter()
        .map(|(field, value)| (field.to_owned(), value))
        .collect()
}
