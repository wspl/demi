//! One change of an entry at a time (`providers.md` § Login and
//! publication): an edit, a deletion, an account change or a test holds the
//! entry while it runs, and a device login into the entry holds it until the
//! login ends. Another change meanwhile is refused as busy.

use std::collections::HashSet;
use std::sync::{Arc, Mutex, PoisonError};

use demi_web_api::ids::ProviderId;

/// The entries a change holds.
#[derive(Debug, Default)]
pub(crate) struct ProviderOperations {
    /// A `std` mutex: edge threads reserve and release, and no section
    /// awaits.
    held: Mutex<HashSet<ProviderId>>,
}

/// An entry held for one change; dropping it releases the entry.
#[derive(Debug)]
pub(crate) struct OperationGuard {
    operations: Arc<ProviderOperations>,
    id: ProviderId,
}

impl ProviderOperations {
    /// Holds entry `id`, or `None` while another change holds it.
    pub(crate) fn reserve(self: &Arc<Self>, id: &ProviderId) -> Option<OperationGuard> {
        let inserted = self
            .held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id.clone());
        inserted.then(|| OperationGuard {
            operations: self.clone(),
            id: id.clone(),
        })
    }
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        self.operations
            .held
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&self.id);
    }
}
