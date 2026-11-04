//! The pages open on view links (`crates/backend-http/src/browser_views.rs`),
//! by each view's id, in memory as the pages are: the view's removal ends
//! them, and so does its integration's browser turned off or deletion, which
//! the shard's changes of the integration say here.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use demi_web_api_protocol::ids::IntegrationId;
use tokio_util::sync::CancellationToken;

#[derive(Default)]
pub struct OpenViews(Mutex<HashMap<String, Opened>>);

struct Opened {
    integration: IntegrationId,
    removed: CancellationToken,
    withdrawn: CancellationToken,
    pages: usize,
}

impl OpenViews {
    /// A page open on view `id` of `integration`, until it is dropped.
    pub fn open(self: &Arc<Self>, id: &str, integration: IntegrationId) -> OpenView {
        let mut open = self.0.lock().expect("the open views are intact");
        let opened = open.entry(id.to_owned()).or_insert_with(|| Opened {
            integration,
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

    /// Ends each page open on view `id`.
    pub fn remove(&self, id: &str) {
        let removed = self.0.lock().expect("the open views are intact").remove(id);
        if let Some(opened) = removed {
            opened.removed.cancel();
        }
    }

    /// Ends each page open on a view of `integration`, whose views no longer
    /// open: its browser was turned off, or it is being deleted.
    pub(crate) fn withdraw(&self, integration: &IntegrationId) {
        self.0.lock().expect("the open views are intact").retain(|_, opened| {
            let kept = opened.integration != *integration;
            if !kept {
                opened.withdrawn.cancel();
            }
            kept
        });
    }
}

/// A page open on a view.
pub struct OpenView {
    views: Arc<OpenViews>,
    id: String,
    /// Cancelled once the view is removed.
    pub removed: CancellationToken,
    /// Cancelled once the view no longer opens.
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
