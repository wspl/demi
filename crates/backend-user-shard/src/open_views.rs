//! The pages open on view links (`crates/backend-http/src/view_links.rs`),
//! by each link's id, in memory as the pages are: the link's removal ends
//! them, and so does its conversation's deletion. A tab's link also ends with
//! its integration's browser turned off, and every link with its
//! integration's deletion, which the shard's changes of the integration say
//! here.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use demi_web_api_protocol::ids::{ConversationId, IntegrationId};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub struct OpenViews(Mutex<HashMap<String, Opened>>);

struct Opened {
    integration: IntegrationId,
    conversation: ConversationId,
    /// It shows a tab of the conversation's browser.
    tab: bool,
    removed: CancellationToken,
    withdrawn: CancellationToken,
    pages: usize,
}

impl OpenViews {
    /// A page open on link `id` of `integration`'s `conversation`, of a tab
    /// of its browser as `tab` says, until it is dropped.
    pub fn open(self: &Arc<Self>, id: &str, integration: IntegrationId, conversation: ConversationId, tab: bool) -> OpenView {
        let mut open = self.0.lock().expect("the open views are intact");
        let opened = open.entry(id.to_owned()).or_insert_with(|| Opened {
            integration,
            conversation,
            tab,
            removed: CancellationToken::new(),
            withdrawn: CancellationToken::new(),
            pages: 0,
        });
        opened.pages += 1;
        OpenView {
            views: self.clone(),
            id: id.to_owned(),
            removed: opened.removed.clone(),
            withdrawn: opened.withdrawn.clone(),
        }
    }

    /// Ends each page open on link `id`.
    pub fn remove(&self, id: &str) {
        let removed = self.0.lock().expect("the open views are intact").remove(id);
        if let Some(opened) = removed {
            opened.removed.cancel();
        }
    }

    /// Ends each page open on a link of `conversation`, which is being
    /// deleted, as its removal would.
    pub(crate) fn remove_conversation(&self, conversation: &ConversationId) {
        self.0.lock().expect("the open views are intact").retain(|_, opened| {
            let kept = opened.conversation != *conversation;
            if !kept {
                opened.removed.cancel();
            }
            kept
        });
    }

    /// Ends each page open on a link of `integration` that no longer opens:
    /// a tab's once its browser was turned off, as `tabs` says, or every
    /// one as it is being deleted.
    pub(crate) fn withdraw(&self, integration: &IntegrationId, tabs: bool) {
        self.0.lock().expect("the open views are intact").retain(|_, opened| {
            let kept = opened.integration != *integration || (tabs && !opened.tab);
            if !kept {
                opened.withdrawn.cancel();
            }
            kept
        });
    }
}

/// A page open on a view link.
pub struct OpenView {
    views: Arc<OpenViews>,
    id: String,
    /// Cancelled once the link is removed.
    pub removed: CancellationToken,
    /// Cancelled once the link no longer opens.
    pub withdrawn: CancellationToken,
}

impl Drop for OpenView {
    fn drop(&mut self) {
        // An ended page's entry is gone already; one of the same id is a
        // later page's.
        if self.removed.is_cancelled() || self.withdrawn.is_cancelled() {
            return;
        }
        let mut open = self.views.0.lock().expect("the open views are intact");
        if let Some(opened) = open.get_mut(&self.id) {
            opened.pages -= 1;
            if opened.pages == 0 {
                open.remove(&self.id);
            }
        }
    }
}
