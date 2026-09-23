//! A credential pool held in memory: the accounts of a login before it
//! completes, which the backend then stores in one transaction
//! (`providers.md` § Login and publication), and the pool of tests.

use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, PoisonError},
};

use futures_util::future::BoxFuture;

use crate::credentials::{
    AccountDocument, AccountMeta, CredentialPool, PoolError, RefreshGates, RefreshPermit,
    Revision,
};

/// A pool held in memory, with refresh turns of its own. Clones share the
/// pool.
#[derive(Debug, Clone, Default)]
pub struct MemoryCredentialPool {
    shared: Arc<Shared>,
}

#[derive(Debug, Default)]
struct Shared {
    /// A `std` mutex: the pool is used from any thread, and every section
    /// reads or writes the map and never awaits.
    held: Mutex<Held>,
    gates: RefreshGates,
}

#[derive(Debug, Default)]
struct Held {
    accounts: BTreeMap<String, Entry>,
    active: Option<String>,
}

#[derive(Debug)]
struct Entry {
    meta: AccountMeta,
    text: String,
    version: u64,
}

impl MemoryCredentialPool {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every account with its secret document, ordered by id, for whoever
    /// stores a completed login.
    pub fn entries(&self) -> Vec<(AccountMeta, String)> {
        self.shared
            .lock()
            .accounts
            .values()
            .map(|entry| (entry.meta.clone(), entry.text.clone()))
            .collect()
    }
}

impl Shared {
    fn lock(&self) -> std::sync::MutexGuard<'_, Held> {
        // A section never panics halfway through a change, so a poisoned
        // lock holds consistent state.
        self.held.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl CredentialPool for MemoryCredentialPool {
    fn list(&self) -> BoxFuture<'_, Result<Vec<AccountMeta>, PoolError>> {
        let accounts = self
            .shared
            .lock()
            .accounts
            .values()
            .map(|entry| entry.meta.clone())
            .collect();
        Box::pin(async { Ok(accounts) })
    }

    fn meta<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<Option<AccountMeta>, PoolError>> {
        let meta = self.shared.lock().accounts.get(id).map(|entry| entry.meta.clone());
        Box::pin(async { Ok(meta) })
    }

    fn active(&self) -> BoxFuture<'_, Result<Option<String>, PoolError>> {
        let active = self.shared.lock().active.clone();
        Box::pin(async { Ok(active) })
    }

    fn set_active<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), PoolError>> {
        let result = {
            let mut held = self.shared.lock();
            if held.accounts.contains_key(id) {
                held.active = Some(id.to_owned());
                Ok(())
            } else {
                Err(PoolError::NotFound(id.to_owned()))
            }
        };
        Box::pin(async { result })
    }

    fn write(&self, meta: AccountMeta, secret: String) -> BoxFuture<'_, Result<(), PoolError>> {
        {
            let mut held = self.shared.lock();
            let version = held.accounts.get(&meta.id).map_or(0, |entry| entry.version) + 1;
            let entry = Entry {
                meta: meta.clone(),
                text: secret,
                version,
            };
            held.accounts.insert(meta.id, entry);
        }
        Box::pin(async { Ok(()) })
    }

    fn document(&self, id: &str) -> Box<dyn AccountDocument> {
        Box::new(MemoryDocument {
            shared: self.shared.clone(),
            id: id.to_owned(),
            name: format!("account {id}"),
        })
    }

    fn remove<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), PoolError>> {
        {
            let mut held = self.shared.lock();
            held.accounts.remove(id);
            if held.active.as_deref() == Some(id) {
                held.active = None;
            }
        }
        Box::pin(async { Ok(()) })
    }
}

/// One account's document in a memory pool.
struct MemoryDocument {
    shared: Arc<Shared>,
    id: String,
    name: String,
}

impl AccountDocument for MemoryDocument {
    fn name(&self) -> &str {
        &self.name
    }

    fn read(&self) -> BoxFuture<'_, Result<Option<Revision>, PoolError>> {
        let revision = self.shared.lock().accounts.get(&self.id).map(|entry| Revision {
            text: entry.text.clone(),
            version: entry.version,
        });
        Box::pin(async { Ok(revision) })
    }

    fn replace(&self, text: String, version: u64) -> BoxFuture<'_, Result<bool, PoolError>> {
        let kept = {
            let mut held = self.shared.lock();
            match held.accounts.get_mut(&self.id) {
                Some(entry) if entry.version == version => {
                    entry.text = text;
                    entry.version += 1;
                    true
                }
                _ => false,
            }
        };
        Box::pin(async move { Ok(kept) })
    }

    fn refresh_turn(&self) -> BoxFuture<'_, RefreshPermit> {
        Box::pin(self.shared.gates.turn(&self.id))
    }
}
