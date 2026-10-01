//! Attachment and transcript media (`storage.md` § Attachment and transcript
//! media, § The object store): each user's bytes under `blobs/<user>/`, named
//! by their SHA-256 in lowercase hexadecimal. A name identifies bytes only
//! within its user's namespace, so knowing another user's hash reaches
//! nothing of theirs. The backend's record of blob uses (`storage.md`
//! § Collecting blobs) is kept here, beside the puts that record them and the
//! deletions they hold back.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use bytes::Bytes;
use demi_agent_store::StoreError;
use demi_agent_store::media::BlobStore;
use demi_shared_types::{B64Bytes, BlobRef, Clock, Timestamp};
use demi_web_api_protocol::ids::UserId;
use futures_util::StreamExt as _;
use futures_util::future::LocalBoxFuture;
use jiff::SignedDuration;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt as _, PutMode, PutOptions, PutPayload};
use tokio::sync::watch;

use crate::ObjectError;

/// Every user's blob namespace, with the backend's record of blob uses.
#[derive(Clone)]
pub struct BlobStores {
    objects: Arc<dyn ObjectStore>,
    uses: Arc<BlobUses>,
}

impl BlobStores {
    /// The namespaces in `objects`, whose uses are timed by `clock`.
    pub fn new(objects: Arc<dyn ObjectStore>, clock: Arc<dyn Clock>) -> Self {
        Self {
            objects,
            uses: Arc::new(BlobUses::new(clock)),
        }
    }

    /// `user`'s namespace: the signed-in user's for uploads and downloads,
    /// the conversation owner's for transcript media.
    pub fn for_user(&self, user: &UserId) -> UserBlobs {
        UserBlobs {
            objects: self.objects.clone(),
            namespace: Path::from_iter(["blobs", user.as_str()]),
            user: user.clone(),
            uses: self.uses.clone(),
        }
    }
}

/// One user's blobs.
#[derive(Clone)]
pub struct UserBlobs {
    objects: Arc<dyn ObjectStore>,
    namespace: Path,
    user: UserId,
    uses: Arc<BlobUses>,
}

/// A blob of a namespace, as its listing finds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBlob {
    pub blob: BlobRef,
    /// When its object was written.
    pub written: Timestamp,
}

impl UserBlobs {
    /// Stores `bytes` and answers their name (`storage.md` § The object
    /// store). A name always names the same bytes, so a blob the namespace
    /// holds already is success: the put asks first whether it exists, and
    /// sends its bytes only when it does not, so repeated uploads of one file
    /// store it once and send it once. The put uses the blob, as recorded
    /// before it asks; a put of a blob being deleted waits until the deletion
    /// has ended and then stores the bytes again (`storage.md` § Collecting
    /// blobs).
    pub async fn put(&self, bytes: Bytes) -> Result<BlobRef, ObjectError> {
        // Hashing an upload of 25 MiB takes tens of milliseconds, which would
        // hold an async thread that long.
        let (bytes, blob) = tokio::task::spawn_blocking(move || {
            let blob = BlobRef::of(&bytes);
            (bytes, blob)
        })
        .await?;
        self.uses.put(&self.user, &blob).await;
        let location = self.location(&blob);
        match self.objects.head(&location).await {
            Ok(_) => return Ok(blob),
            Err(object_store::Error::NotFound { .. }) => {}
            Err(error) => return Err(error.into()),
        }
        let create = PutOptions {
            mode: PutMode::Create,
            ..PutOptions::default()
        };
        match self.objects.put_opts(&location, PutPayload::from_bytes(bytes), create).await {
            // A put of the same bytes may have created it since the HEAD.
            Ok(_) | Err(object_store::Error::AlreadyExists { .. }) => Ok(blob),
            Err(error) => Err(error.into()),
        }
    }

    /// The bytes `blob` names, or `None` when this namespace does not hold
    /// them.
    pub async fn get(&self, blob: &BlobRef) -> Result<Option<Bytes>, ObjectError> {
        match self.objects.get(&self.location(blob)).await {
            Ok(object) => Ok(Some(object.bytes().await?)),
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Every blob the namespace holds, with when its object was written: one
    /// listing, which on S3 is one request per 1,000 objects. An object whose
    /// name is no blob's is left out.
    pub async fn list(&self) -> Result<Vec<StoredBlob>, ObjectError> {
        let mut listing = self.objects.list(Some(&self.namespace));
        let mut blobs = Vec::new();
        while let Some(object) = listing.next().await {
            let object = object?;
            let Some(name) = object.location.filename() else {
                continue;
            };
            let Ok(blob) = BlobRef::try_from(name.to_owned()) else {
                tracing::warn!(object = %object.location, "an object in a blob namespace is not a blob");
                continue;
            };
            let written = Timestamp::from_millisecond(object.last_modified.timestamp_millis()).map_err(|error| {
                ObjectError::Corrupt {
                    location: object.location.to_string(),
                    field: "last modified time",
                    reason: error.to_string(),
                }
            })?;
            blobs.push(StoredBlob { blob, written });
        }
        Ok(blobs)
    }

    /// Deletes `blob` unless something used it within `grace`. The check and
    /// the mark that the blob is being deleted are one step on the record of
    /// uses: from then until the deletion has ended, a put of the blob waits
    /// and a commit that would write a reference to it fails. Answers whether
    /// the blob was deleted.
    pub async fn delete_unused(&self, blob: &BlobRef, grace: SignedDuration) -> Result<bool, ObjectError> {
        let Some(_deleting) = self.uses.mark(&self.user, blob, grace) else {
            return Ok(false);
        };
        match self.objects.delete(&self.location(blob)).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(true),
            Err(error) => Err(error.into()),
        }
    }

    /// Records that a commit writes or removes references to `blobs`, inside
    /// its transaction and before it commits. It is refused when one of them
    /// is being deleted, and the commit must then not commit.
    pub fn commit_uses<'a>(&self, blobs: impl IntoIterator<Item = &'a BlobRef>) -> Result<(), StoreError> {
        self.uses.commit(&self.user, blobs)
    }

    /// Forgets the uses older than `grace`, which hold no deletion back any
    /// more.
    pub fn forget_uses(&self, grace: SignedDuration) {
        self.uses.forget(&self.user, grace);
    }

    fn location(&self, blob: &BlobRef) -> Path {
        self.namespace.clone().join(blob.as_str())
    }
}

/// The namespace as a session reaches it through its tree store: the
/// conversation owner's, where a tool's medium is stored as its result enters
/// the transcript and the replayed media are read back (`runtime.md`
/// § Media).
impl BlobStore for UserBlobs {
    fn put(&self, bytes: B64Bytes) -> LocalBoxFuture<'_, Result<BlobRef, StoreError>> {
        Box::pin(async move {
            UserBlobs::put(self, bytes.into_bytes())
                .await
                .map_err(|error| StoreError::Failed(error.to_string()))
        })
    }

    fn get<'a>(&'a self, blob: &'a BlobRef) -> LocalBoxFuture<'a, Result<Option<B64Bytes>, StoreError>> {
        Box::pin(async move {
            let bytes = UserBlobs::get(self, blob)
                .await
                .map_err(|error| StoreError::Failed(error.to_string()))?;
            Ok(bytes.map(B64Bytes::from))
        })
    }
}

/// When each blob was last used, per user, and which blobs are being
/// deleted (`storage.md` § Collecting blobs). A put uses its blob as it
/// starts, and a commit uses each blob it writes or removes a reference to
/// before it commits; a collection deletes a blob only when its last use is
/// older than the grace. The record lives with the process, and so does
/// everything it protects, such as a block not saved yet.
struct BlobUses {
    clock: Arc<dyn Clock>,
    /// A `std` mutex: uploads at the edge, the collectors on the shard
    /// threads and the commits on the conversation databases' writer threads
    /// share it, and each section only reads or changes the maps.
    state: Mutex<Uses>,
    /// Bumped as each deletion ends, for the puts that wait for one.
    deleted: watch::Sender<u64>,
}

#[derive(Default)]
struct Uses {
    last: HashMap<UserId, HashMap<BlobRef, Timestamp>>,
    deleting: HashSet<(UserId, BlobRef)>,
}

impl BlobUses {
    fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            clock,
            state: Mutex::new(Uses::default()),
            deleted: watch::Sender::new(0),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Uses> {
        // No section panics while it holds the lock, so a poisoned one still
        // holds whole maps.
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Records a put of `blob` as it starts, once no deletion of it is under
    /// way.
    async fn put(&self, user: &UserId, blob: &BlobRef) {
        let key = (user.clone(), blob.clone());
        loop {
            // Subscribed before the check, so a deletion that ends after it
            // wakes the wait.
            let mut ended = self.deleted.subscribe();
            {
                let mut uses = self.lock();
                if !uses.deleting.contains(&key) {
                    let now = self.clock.now();
                    uses.last.entry(user.clone()).or_default().insert(blob.clone(), now);
                    return;
                }
            }
            // The record keeps its sender while anyone puts.
            let _ = ended.changed().await;
        }
    }

    fn commit<'a>(&self, user: &UserId, blobs: impl IntoIterator<Item = &'a BlobRef>) -> Result<(), StoreError> {
        let blobs: Vec<&BlobRef> = blobs.into_iter().collect();
        if blobs.is_empty() {
            return Ok(());
        }
        let mut uses = self.lock();
        let deleting = blobs
            .iter()
            .find(|blob| uses.deleting.contains(&(user.clone(), (**blob).clone())));
        if let Some(blob) = deleting {
            return Err(StoreError::Failed(format!("blob {blob} is being deleted")));
        }
        let now = self.clock.now();
        let last = uses.last.entry(user.clone()).or_default();
        for blob in blobs {
            last.insert(blob.clone(), now);
        }
        Ok(())
    }

    /// Marks `blob` as being deleted when its last use, if any, is older than
    /// `grace`; the mark goes when the answer is dropped.
    fn mark(&self, user: &UserId, blob: &BlobRef, grace: SignedDuration) -> Option<DeletionMark<'_>> {
        let now = self.clock.now();
        let mut uses = self.lock();
        let recent = uses
            .last
            .get(user)
            .and_then(|last| last.get(blob))
            .is_some_and(|used| now.to_jiff().duration_since(used.to_jiff()) <= grace);
        let key = (user.clone(), blob.clone());
        if recent || !uses.deleting.insert(key.clone()) {
            return None;
        }
        Some(DeletionMark { uses: self, key })
    }

    fn forget(&self, user: &UserId, grace: SignedDuration) {
        let now = self.clock.now();
        let mut uses = self.lock();
        let Some(last) = uses.last.get_mut(user) else {
            return;
        };
        last.retain(|_, used| now.to_jiff().duration_since(used.to_jiff()) <= grace);
        if last.is_empty() {
            uses.last.remove(user);
        }
    }
}

/// A blob being deleted: dropping it ends the deletion, and the puts that
/// waited for it go on.
struct DeletionMark<'a> {
    uses: &'a BlobUses,
    key: (UserId, BlobRef),
}

impl Drop for DeletionMark<'_> {
    fn drop(&mut self) {
        self.uses.lock().deleting.remove(&self.key);
        self.uses.deleted.send_modify(|ended| *ended = ended.wrapping_add(1));
    }
}

#[cfg(test)]
mod tests {
    use std::fmt;
    use std::time::Duration;

    use futures_util::stream::BoxStream;
    use object_store::memory::InMemory;
    use object_store::{
        CopyOptions, GetOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, PutMultipartOptions, PutResult,
        RenameOptions, Result,
    };
    use tokio::sync::Notify;

    use super::*;

    const DAY: SignedDuration = SignedDuration::from_hours(24);

    /// Wall-clock time the test sets.
    struct TestClock(Mutex<Timestamp>);

    impl Clock for TestClock {
        fn now(&self) -> Timestamp {
            *self.0.lock().unwrap()
        }
    }

    /// An object store in memory whose deletions wait, once they reached it,
    /// until the test lets them go.
    #[derive(Debug)]
    struct HeldDeletions {
        inner: InMemory,
        reached: Arc<Notify>,
        go: Arc<Notify>,
    }

    impl fmt::Display for HeldDeletions {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "HeldDeletions({})", self.inner)
        }
    }

    #[async_trait::async_trait]
    impl ObjectStore for HeldDeletions {
        async fn put_opts(&self, location: &Path, payload: PutPayload, opts: PutOptions) -> Result<PutResult> {
            self.inner.put_opts(location, payload, opts).await
        }

        async fn put_multipart_opts(&self, location: &Path, opts: PutMultipartOptions) -> Result<Box<dyn MultipartUpload>> {
            self.inner.put_multipart_opts(location, opts).await
        }

        async fn get_opts(&self, location: &Path, options: GetOptions) -> Result<GetResult> {
            self.inner.get_opts(location, options).await
        }

        fn delete_stream(&self, locations: BoxStream<'static, Result<Path>>) -> BoxStream<'static, Result<Path>> {
            let (reached, go) = (self.reached.clone(), self.go.clone());
            let held = locations.then(move |location| {
                let (reached, go) = (reached.clone(), go.clone());
                async move {
                    reached.notify_one();
                    go.notified().await;
                    location
                }
            });
            self.inner.delete_stream(held.boxed())
        }

        fn list(&self, prefix: Option<&Path>) -> BoxStream<'static, Result<ObjectMeta>> {
            self.inner.list(prefix)
        }

        async fn list_with_delimiter(&self, prefix: Option<&Path>) -> Result<ListResult> {
            self.inner.list_with_delimiter(prefix).await
        }

        async fn copy_opts(&self, from: &Path, to: &Path, options: CopyOptions) -> Result<()> {
            self.inner.copy_opts(from, to, options).await
        }

        async fn rename_opts(&self, from: &Path, to: &Path, options: RenameOptions) -> Result<()> {
            self.inner.rename_opts(from, to, options).await
        }
    }

    #[tokio::test]
    async fn a_put_that_meets_a_deletion_waits_for_it_and_stores_the_bytes_again() {
        let (reached, go) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
        let objects = Arc::new(HeldDeletions {
            inner: InMemory::new(),
            reached: reached.clone(),
            go: go.clone(),
        });
        let clock = Arc::new(TestClock(Mutex::new(Timestamp::UNIX_EPOCH)));
        let blobs = BlobStores::new(objects, clock.clone()).for_user(&UserId::try_from("ana").unwrap());
        let bytes = Bytes::from_static(b"a screenshot");
        let blob = blobs.put(bytes.clone()).await.unwrap();
        // The put used the blob, which keeps it for the grace.
        assert!(!blobs.delete_unused(&blob, DAY).await.unwrap());

        let later = i64::try_from((DAY * 2).as_millis()).unwrap();
        *clock.0.lock().unwrap() = Timestamp::from_millisecond(later).unwrap();
        let deletion = tokio::spawn({
            let (blobs, blob) = (blobs.clone(), blob.clone());
            async move { blobs.delete_unused(&blob, DAY).await }
        });
        reached.notified().await;
        // While the blob is being deleted, a commit that would name it fails,
        // and a put waits.
        assert!(blobs.commit_uses([&blob]).is_err());
        let put = tokio::spawn({
            let (blobs, bytes) = (blobs.clone(), bytes.clone());
            async move { blobs.put(bytes).await }
        });
        let waiting = async {
            while blobs.uses.deleted.receiver_count() == 0 {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        };
        tokio::time::timeout(Duration::from_secs(10), waiting).await.expect("the put waits for the deletion");
        go.notify_one();
        assert!(deletion.await.unwrap().unwrap());
        // The put stored the bytes again once the deletion ended, so the
        // blob it answered can be read.
        assert_eq!(put.await.unwrap().unwrap(), blob);
        assert_eq!(blobs.get(&blob).await.unwrap(), Some(bytes));
        assert!(blobs.commit_uses([&blob]).is_ok());
    }
}
