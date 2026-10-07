//! The collection of a user's blob namespace (`storage.md` § Deleting a
//! conversation): after a deletion, in the background, a blob that no
//! remaining record of the user names, that was written more than a day ago
//! and that was not put again since, is removed, and so is each upload
//! record of it. The day keeps a blob whose reference is not written yet,
//! since a blob is stored before the row that names it. A user's collections
//! run one at a time; one asked for while another runs starts after it. A collection fails closed: when one of
//! the records that may name a blob cannot be read, it removes nothing.

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
    /// Where the collections stand from now on, for a caller that waits for
    /// them to end.
    #[cfg(feature = "testing")]
    pub(crate) fn subscribe(&self) -> watch::Receiver<CollectionState> {
        self.state.subscribe()
    }
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
                        tracing::info!(user = %shard.user(), removed, "unreferenced blobs removed");
                    }
                    Err(why) => {
                        tracing::warn!(user = %shard.user(), "the collection removed nothing more: {why}");
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
        // A put of a blob that existed wrote nothing, so the blob's age is
        // its first write's; one put again within the day stays too.
        referenced.extend(blobs.put_again_recently());
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
            match blobs.delete(&blob).await {
                Ok(()) => removed.push(blob),
                // The blob stays, and so does its upload record, until a
                // later collection removes both.
                Err(error) => {
                    tracing::warn!(user = %self.user(), %blob, error = &error as &dyn std::error::Error, "a blob was not removed");
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime};

    use bytes::Bytes;
    use demi_backend_database::control::testing;

    use crate::services::Services;
    use crate::shard::{ShardPlacement, ShardPool};

    use super::*;

    /// Writes `bytes` into the user's namespace as a put two days ago left
    /// them, and answers their name.
    fn written_two_days_ago(data: &std::path::Path, user: &str, bytes: &[u8]) -> BlobRef {
        let blob = BlobRef::of(bytes);
        let directory = data.join("blobs").join(user);
        std::fs::create_dir_all(&directory).unwrap();
        let path = directory.join(blob.as_str());
        std::fs::write(&path, bytes).unwrap();
        let two_days = Duration::from_secs(2 * 24 * 60 * 60);
        std::fs::File::options()
            .write(true)
            .open(&path)
            .unwrap()
            .set_modified(SystemTime::now() - two_days)
            .unwrap();
        blob
    }

    #[tokio::test(flavor = "local")]
    async fn a_blob_put_again_within_the_day_stays_though_nothing_names_it() {
        let data = tempfile::tempdir().unwrap();
        let services = Services::start_for_tests(data.path()).await;
        let owner = testing::master(&services.control).await.id;
        let again = written_two_days_ago(data.path(), owner.as_str(), b"put again");
        let left = written_two_days_ago(data.path(), owner.as_str(), b"left alone");
        // A put of a blob the namespace holds writes nothing: the blob keeps
        // the age of its first write.
        let blobs = services.blobs.for_user(&owner);
        assert_eq!(blobs.put(Bytes::from_static(b"put again")).await.unwrap(), again);
        let pool = ShardPool::start(ShardPlacement::Inline, services)
            .await
            .unwrap();
        let removed = pool
            .shards()
            .of(&owner)
            .call(|shard, _| async move { shard.collect_once().await })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(removed, 1);
        let namespace = data.path().join("blobs").join(owner.as_str());
        assert!(namespace.join(again.as_str()).exists());
        assert!(!namespace.join(left.as_str()).exists());
        pool.close().await;
    }
}
