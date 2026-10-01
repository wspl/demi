//! What the command holding a tab's operation lock uses of the tab
//! (`browser.md` § Owners inside the service): its node references, asset
//! inventories and WebMCP tool sets. The data lives here; the commands that
//! use it are written above the tabs.

use std::{
    collections::HashMap,
    ops::{Deref, DerefMut},
};

use chromiumoxide::cdp::browser_protocol::{dom::BackendNodeId, network::LoaderId, page::FrameId};

use crate::driver::operation::{BrowserError, Result};

use crate::tabs::protocol::{AssetKind, NodeRef, WebmcpTool};

/// The session a command holds with a tab's operation lock. It returns to
/// the tab when the command releases the lock.
#[derive(Default)]
pub struct TabSession {
    pub references: References,
    pub assets: Assets,
    pub webmcp: Webmcp,
}

/// A tab's operation lock: one agent command at a time holds it, and with it
/// the tab's session.
pub struct TabGate {
    /// The session while no command holds it. A std mutex, touched only to
    /// take the session out and to put it back, never across an await: a
    /// command holds the session itself, not the mutex.
    session: std::sync::Mutex<Option<Box<TabSession>>>,
}

impl Default for TabGate {
    fn default() -> Self {
        Self {
            session: std::sync::Mutex::new(Some(Box::default())),
        }
    }
}

impl TabGate {
    /// The tab's session, unless another command holds it.
    pub fn try_checkout(&self) -> Option<Checkout<'_>> {
        let session = self.slot().take()?;
        Some(Checkout {
            gate: self,
            session: Some(session),
        })
    }

    fn slot(&self) -> std::sync::MutexGuard<'_, Option<Box<TabSession>>> {
        // Nothing can panic between taking the lock and releasing it.
        self.session.lock().expect("the tab gate is intact")
    }
}

/// A tab's session, held by the command that holds the tab's operation lock.
pub struct Checkout<'a> {
    gate: &'a TabGate,
    session: Option<Box<TabSession>>,
}

impl Deref for Checkout<'_> {
    type Target = TabSession;

    fn deref(&self) -> &TabSession {
        self.session.as_ref().expect("a checkout holds its session")
    }
}

impl DerefMut for Checkout<'_> {
    fn deref_mut(&mut self) -> &mut TabSession {
        self.session.as_mut().expect("a checkout holds its session")
    }
}

impl Drop for Checkout<'_> {
    fn drop(&mut self) {
        *self.gate.slot() = self.session.take();
    }
}

/// The node a reference names, in the document it was observed in.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct Reference {
    pub backend: BackendNodeId,
    pub frame: FrameId,
    pub loader: LoaderId,
}

/// A tab's node references (`browser.md` § Page trees and references): each
/// is `e` and the tab's next number, never given twice in the tab, so a
/// reference to an earlier document is recognized as stale.
#[derive(Default)]
pub struct References {
    by_id: HashMap<NodeRef, Reference>,
    by_node: HashMap<Reference, NodeRef>,
    /// The number the tab gave out last.
    last: u64,
}

impl References {
    /// Forgets the references of the document the tab left; their numbers
    /// are not given out again.
    pub fn invalidate(&mut self) {
        self.by_id.clear();
        self.by_node.clear();
    }

    /// Keeps the references whose node `keep` accepts.
    pub fn retain(&mut self, mut keep: impl FnMut(&Reference) -> bool) {
        self.by_id.retain(|_, reference| keep(reference));
        let by_id = &self.by_id;
        self.by_node.retain(|_, id| by_id.contains_key(id));
    }

    /// The node `id` names, while the tab keeps it.
    pub fn get(&self, id: &NodeRef) -> Option<&Reference> {
        self.by_id.get(id)
    }

    /// The reference to `reference`: the one given before, or the tab's next.
    pub fn issue(&mut self, reference: Reference) -> Result<NodeRef> {
        if let Some(id) = self.by_node.get(&reference) {
            return Ok(id.clone());
        }
        if self.by_id.len() >= 10_000 {
            return Err(BrowserError::Configuration(
                "browser reference limit reached for this document".into(),
            ));
        }
        self.last += 1;
        let id = NodeRef::numbered(self.last);
        self.by_id.insert(id.clone(), reference.clone());
        self.by_node.insert(reference, id.clone());
        Ok(id)
    }
}

/// A tab's asset inventories (`browser.md` § Content and assets), for the
/// documents they were taken in.
#[derive(Default)]
pub struct Assets {
    pub documents: HashMap<FrameId, LoaderId>,
    pub inventories: HashMap<String, Inventory>,
}

/// One inventory's assets, by the handle a listing gave it.
pub struct Inventory {
    pub assets: Vec<Asset>,
}

pub struct Asset {
    pub id: String,
    pub kind: AssetKind,
    pub mime: String,
    pub source: AssetSource,
}

/// Where an asset's bytes come from.
pub enum AssetSource {
    Resource {
        page: chromiumoxide::Page,
        frame: FrameId,
        url: String,
    },
    Svg(String),
}

/// A tab's WebMCP tool sets (`browser.md` § Capabilities and WebMCP), for
/// the document they were declared in.
#[derive(Default)]
pub struct Webmcp {
    pub document: Option<LoaderId>,
    pub key: Option<String>,
    pub tools: Option<ToolSet>,
}

/// The tools a listing declared, by the handle it gave them.
pub struct ToolSet {
    pub handle: String,
    pub entries: Vec<WebmcpTool>,
}
