//! One change of an entry at a time
//! (`crates/backend-providers/src/vault/logins.rs`): an edit, a deletion, an
//! account change or a test holds the
//! entry while it runs, and a login into the entry holds it until the login
//! ends. A login that makes a new entry holds that entry, the owner's new
//! entry of its family, the same way. Another change meanwhile is refused as
//! busy, and a start refused by its starter's own login names that login.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use demi_web_api_protocol::ids::{LoginId, ProviderId, UserId};

/// What a change holds.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Held {
    Entry(ProviderId),
    /// The owner's entry of the family, which a login is making.
    NewEntry { owner: UserId, family: String },
}

/// The login a reservation is for: its ID and the user who started it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HeldBy {
    pub login: LoginId,
    pub starter: UserId,
}

/// What each change holds, and the login when a login holds it.
#[derive(Debug, Default)]
pub struct ProviderOperations {
    /// A `std` mutex: edge threads reserve and release, and no section
    /// awaits.
    held: Mutex<HashMap<Held, Option<HeldBy>>>,
}

/// Something held for one change; dropping it releases it.
#[derive(Debug)]
pub struct OperationGuard {
    operations: Arc<ProviderOperations>,
    held: Held,
}

impl ProviderOperations {
    fn lock(&self) -> MutexGuard<'_, HashMap<Held, Option<HeldBy>>> {
        // A section only inserts or removes one key, so a poisoned map is
        // whole.
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Holds entry `id` for a change other than a login, or `None` while
    /// another change holds it.
    pub fn reserve(self: &Arc<Self>, id: &ProviderId) -> Option<OperationGuard> {
        self.hold(Held::Entry(id.clone()), None).ok()
    }

    /// Holds `held` for `by`'s login; while another change holds it, the
    /// error is the login that does, if any.
    pub fn hold(self: &Arc<Self>, held: Held, by: Option<HeldBy>) -> Result<OperationGuard, Option<HeldBy>> {
        let mut map = self.lock();
        if let Some(holder) = map.get(&held) {
            return Err(holder.clone());
        }
        map.insert(held.clone(), by);
        Ok(OperationGuard {
            operations: self.clone(),
            held,
        })
    }
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        self.operations.lock().remove(&self.held);
    }
}
