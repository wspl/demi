//! One holder at a time, in arrival order. A permit is owned, so the work it
//! admits can move into a task.

use std::{
    collections::HashMap,
    hash::Hash,
    sync::{Arc, Mutex, PoisonError},
};

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

/// Admits one holder at a time, in arrival order.
#[derive(Clone, Debug)]
pub struct SerialGate {
    permits: Arc<Semaphore>,
    /// How many callers wait for their turn, for a test.
    #[cfg(feature = "testing")]
    waiting: Arc<tokio::sync::watch::Sender<usize>>,
}

impl SerialGate {
    pub fn new() -> Self {
        Self {
            permits: Arc::new(Semaphore::new(1)),
            #[cfg(feature = "testing")]
            waiting: Arc::new(tokio::sync::watch::Sender::new(0)),
        }
    }

    /// Holds the gate once every earlier caller has released it.
    pub async fn acquire(&self) -> SerialPermit {
        let acquire = self.permits.clone().acquire_owned();
        #[cfg(feature = "testing")]
        let acquire = crate::activity::counted(&self.waiting, acquire);
        let permit = acquire.await.expect("a gate's semaphore is never closed");
        SerialPermit(permit)
    }

    /// Holds the gate now, or none while someone holds or waits for it.
    pub fn try_acquire(&self) -> Option<SerialPermit> {
        self.permits.clone().try_acquire_owned().ok().map(SerialPermit)
    }

    /// How many callers wait for their turn now (`testing`).
    #[cfg(feature = "testing")]
    pub fn waiting(&self) -> tokio::sync::watch::Receiver<usize> {
        self.waiting.subscribe()
    }
}

impl Default for SerialGate {
    fn default() -> Self {
        Self::new()
    }
}

/// The turn a [`SerialGate`] gave; dropping it gives the next caller its turn.
#[derive(Debug)]
pub struct SerialPermit(#[allow(dead_code, reason = "held for its drop")] OwnedSemaphorePermit);

/// Gates keyed by, for example, a conversation: callers of one key take
/// turns, callers of different keys do not wait for each other.
#[derive(Debug)]
pub struct KeyedSerialGate<K> {
    /// A `std` mutex: each section only looks up, inserts or removes an entry
    /// and never awaits, and permits drop on any thread.
    gates: Arc<Mutex<HashMap<K, Arc<Semaphore>>>>,
}

impl<K> Clone for KeyedSerialGate<K> {
    fn clone(&self) -> Self {
        Self {
            gates: self.gates.clone(),
        }
    }
}

impl<K: Eq + Hash + Clone> KeyedSerialGate<K> {
    pub fn new() -> Self {
        Self {
            gates: Arc::default(),
        }
    }

    /// Holds `key`'s turn once every earlier caller of the same key has
    /// released it.
    pub async fn acquire(&self, key: K) -> KeyedPermit<K> {
        let semaphore = self
            .gates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Semaphore::new(1)))
            .clone();
        let permit = semaphore
            .acquire_owned()
            .await
            .expect("a gate's semaphore is never closed");
        KeyedPermit {
            gates: self.gates.clone(),
            key,
            permit: Some(permit),
        }
    }

    /// Holds `key`'s turn now, or none while another caller holds or waits
    /// for it.
    pub fn try_acquire(&self, key: K) -> Option<KeyedPermit<K>> {
        let mut gates = self.gates.lock().unwrap_or_else(PoisonError::into_inner);
        let semaphore = gates
            .entry(key.clone())
            .or_insert_with(|| Arc::new(Semaphore::new(1)))
            .clone();
        // Tried under the map's lock, where a permit is released: a refused
        // caller drops its handle before a holder's release looks at the
        // entry, which goes once nobody holds or waits for the key.
        let permit = semaphore.try_acquire_owned().ok()?;
        Some(KeyedPermit {
            gates: self.gates.clone(),
            key,
            permit: Some(permit),
        })
    }

    /// How many keys have a holder or a waiter.
    pub fn len(&self) -> usize {
        self.gates
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<K: Eq + Hash + Clone> Default for KeyedSerialGate<K> {
    fn default() -> Self {
        Self::new()
    }
}

/// One key's turn; dropping it gives the next caller of the key its turn,
/// and removes the key's gate when nobody holds or waits for it.
#[derive(Debug)]
pub struct KeyedPermit<K: Eq + Hash> {
    gates: Arc<Mutex<HashMap<K, Arc<Semaphore>>>>,
    key: K,
    permit: Option<OwnedSemaphorePermit>,
}

impl<K: Eq + Hash> Drop for KeyedPermit<K> {
    fn drop(&mut self) {
        let mut gates = self.gates.lock().unwrap_or_else(PoisonError::into_inner);
        // Released under the map's lock: a waiter still holds its own handle
        // on the semaphore, so the entry stays while anyone waits.
        drop(self.permit.take());
        if gates
            .get(&self.key)
            .is_some_and(|semaphore| Arc::strong_count(semaphore) == 1)
        {
            gates.remove(&self.key);
        }
    }
}
