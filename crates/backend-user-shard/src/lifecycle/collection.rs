//! The collection of a user's blob namespace
//! (`crates/backend-database/src/deletions.rs`).
//!
//! After a deletion, in the background, a blob is removed, and so is each upload
//! record of it, when:
//!
//! - no remaining record of the user names it;
//! - it was written more than a day ago;
//! - it was not put again since.
//!
//! The day keeps a blob whose reference is not written yet, since a blob is
//! stored before the row that names it.
//!
//! A user's collections run one at a time; one asked for while another runs
//! starts after it. A collection fails closed: when one of the records that may
//! name a blob cannot be read, it removes nothing.

use demi_backend_blobs::blobs::GRACE_MS;
use demi_backend_database::references::conversation_blobs;
use demi_shared_types::{BlobRef, Timestamp};
use tokio::sync::watch;

use crate::shard::Shard;

/// Where the user's collections stand.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum CollectionState {
    #[default]
    Idle,
    Running,
    /// One runs, and another starts after it.
    Again,
}

/// The user's collections, one at a time.
pub(crate) struct BlobCollection {
    state: watch::Sender<CollectionState>,
}

impl Default for BlobCollection {
    fn default() -> Self {
        Self {
            state: watch::Sender::new(CollectionState::Idle),
        }
    }
}

impl BlobCollection {
}

impl Shard {
    /// Collects the user's blob namespace in the background, or once more
    /// after the collection that runs.
    pub(crate) fn collect_blobs(&self) {
        // A closing shard starts nothing; the next deletion collects.
        if self.is_closing() {
            return;
        }
        let state = &self.blob_collection().state;
        if *state.borrow() != CollectionState::Idle {
            state.send_replace(CollectionState::Again);
            return;
        }
        state.send_replace(CollectionState::Running);
        let shard = self.this();
        self.tasks().spawn_local(async move {
            loop {
                match shard.collect_once().await {
                    Ok(0) => {}
                    Ok(removed) => {
                        tracing::info!(event = "blobs.collected", user = %shard.user(), removed, "unreferenced blobs removed");
                    }
                    Err(why) => {
                        tracing::warn!(event = "blobs.collect_stopped", user = %shard.user(), %why, "the collection removed nothing more");
                    }
                }
                let state = &shard.blob_collection().state;
                // A closing shard leaves the next one to the next deletion.
                if *state.borrow() == CollectionState::Again && !shard.is_closing() {
                    state.send_replace(CollectionState::Running);
                    continue;
                }
                state.send_replace(CollectionState::Idle);
                break;
            }
        });
    }

    /// One collection; answers how many blobs it removed.
    async fn collect_once(&self) -> Result<usize, String> {
        let services = self.services();
        let blobs = services.blobs.for_user(self.user());
        let now = services.clock.now();
        let cutoff = Timestamp::from_millisecond(now.as_millisecond() - GRACE_MS)
            .map_err(|error| error.to_string())?;
        let stored = blobs
            .list()
            .await
            .map_err(|error| format!("the blob namespace cannot be listed: {error}"))?;
        let old: Vec<BlobRef> = stored
            .into_iter()
            .filter(|stored| stored.written < cutoff)
            .map(|stored| stored.blob)
            .collect();
        if old.is_empty() {
            return Ok(0);
        }
        let control = services
            .control
            .blob_references(self.user().clone(), cutoff)
            .await
            .map_err(|error| format!("the control records cannot be read: {error}"))?;
        let mut referenced = control.blobs;
        for id in control.databases {
            let named = services
                .conversations
                .read(&id, conversation_blobs)
                .await
                .map_err(|error| {
                    format!("the database of conversation {id} cannot be read: {error}")
                })?;
            referenced.extend(named.into_iter().flatten());
        }
        let mut removed = Vec::new();
        for blob in old.into_iter().filter(|blob| !referenced.contains(blob)) {
            if self.is_closing() {
                break;
            }
            // A put of a blob that existed wrote nothing, so the blob's age
            // is its first write's; one put again within the day stays too.
            match blobs.delete_unless_put_again(&blob).await {
                Ok(true) => removed.push(blob),
                Ok(false) => {}
                // The blob stays, and so does its upload record, until a
                // later collection removes both.
                Err(error) => {
                    tracing::warn!(event = "blob.remove_failed", user = %self.user(), %blob, error = &error as &dyn std::error::Error, "a blob was not removed");
                }
            }
        }
        let count = removed.len();
        services
            .control
            .remove_uploads(self.user().clone(), removed)
            .await
            .map_err(|error| format!("the upload records of removed blobs stay: {error}"))?;
        Ok(count)
    }
}
