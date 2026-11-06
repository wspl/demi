//! Each user's core store, under `users/<id>/` in the server's data directory.
//!
//! A user's store opens at its first use and stays open: its database is one
//! connection on one thread, and its conversation databases open and close on
//! their own.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};

use demi_core_store::StorageError;
use demi_core_store::conversations::ConversationBlobs;
use demi_core_store::store::CoreStore;
use demi_shared_types::Clock;
use demi_web_api_protocol::ids::UserId;
use tokio::sync::OnceCell;

use super::control::ControlService;

/// The name of the directory of the users' data in the server's.
pub const USER_DATA: &str = "users";

/// The directory of `user`'s core in the data directory `data`.
pub fn user_directory(data: &Path, user: &UserId) -> PathBuf {
    data.join(USER_DATA).join(user.as_str())
}

/// Where the migrations of a user's conversation databases read the blobs they
/// name.
pub trait UserBlobs: Send + Sync {
    fn of(&self, user: &UserId) -> Arc<ConversationBlobs>;
}

/// Each user's core store. Cloning it is cheap.
#[derive(Clone)]
pub struct CoreStores(Arc<Stores>);

struct Stores {
    data: PathBuf,
    clock: Arc<dyn Clock>,
    blobs: Arc<dyn UserBlobs>,
    open: Mutex<HashMap<UserId, Arc<OnceCell<CoreStore>>>>,
}

impl CoreStores {
    /// The stores in the data directory `data`.
    pub fn new(data: PathBuf, clock: Arc<dyn Clock>, blobs: Arc<dyn UserBlobs>) -> Self {
        Self(Arc::new(Stores {
            data,
            clock,
            blobs,
            open: Mutex::new(HashMap::new()),
        }))
    }

    /// `user`'s store, opened at its first use.
    pub async fn of(&self, user: &UserId) -> Result<CoreStore, StorageError> {
        let cell = self
            .0
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(user.clone())
            .or_default()
            .clone();
        cell.get_or_try_init(|| async {
            let directory = user_directory(&self.0.data, user);
            CoreStore::open(&directory, user.clone(), self.0.clock.clone(), self.0.blobs.of(user)).await
        })
        .await
        .cloned()
    }

    /// Every account's store, each with its user.
    pub async fn all(&self, control: &ControlService) -> Result<Vec<(UserId, CoreStore)>, StorageError> {
        let mut all = Vec::new();
        for user in control.user_ids().await? {
            let store = self.of(&user).await?;
            all.push((user, store));
        }
        Ok(all)
    }

    /// Closes every open store, and answers what failed.
    pub async fn close(&self) -> Vec<StorageError> {
        let open: Vec<_> = self
            .0
            .open
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .filter_map(|cell| cell.get().cloned())
            .collect();
        let mut failures = Vec::new();
        for store in open {
            failures.extend(store.close().await);
        }
        failures
    }
}
