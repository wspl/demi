//! Attachment and transcript media (`storage.md` § Attachment and transcript
//! media, § The object store): each user's bytes under `blobs/<user>/`, named
//! by their SHA-256 in lowercase hexadecimal. A name identifies bytes only
//! within its user's namespace, so knowing another user's hash reaches
//! nothing of theirs. A collection of the namespace lists it and deletes
//! the blobs nothing names (`storage.md` § Deleting a conversation); every
//! put of a blob the namespace holds already is remembered for a day, since
//! it writes nothing that would show its age.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use bytes::Bytes;
use demi_agent_store::StoreError;
use demi_agent_store::media::BlobStore;
use demi_shared_types::{B64Bytes, BlobRef, Clock, Timestamp};
use demi_web_api_protocol::ids::UserId;
use futures_util::StreamExt as _;
use futures_util::future::LocalBoxFuture;
use object_store::path::Path;
use object_store::{GetOptions, ObjectStore, ObjectStoreExt as _, PutMode, PutOptions, PutPayload};

use crate::ObjectError;

/// The most bytes of a blob read for what its opening tells: the media type
/// its bytes show and a text file's snippet both come from a file's first
/// bytes (`web-api.md` § Uploads and media).
pub const OPENING_BYTES: u64 = 64 * 1024;

/// A blob's first [`OPENING_BYTES`], or all of a smaller one, and its size.
#[derive(Debug, Clone)]
pub struct BlobOpening {
    pub bytes: Bytes,
    pub size: u64,
}

/// A blob of a namespace, as its listing finds it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredBlob {
    pub blob: BlobRef,
    /// When its object was written. A put of a blob the namespace holds
    /// writes nothing, so this is when its bytes were first stored.
    pub written: Timestamp,
}

/// How long a blob stays whatever names it, in milliseconds: a day after its
/// object was written or it was put again (`storage.md` § Deleting a
/// conversation). A blob is stored before the row that names it, and the
/// day covers the wait.
pub const GRACE_MS: i64 = 24 * 60 * 60 * 1000;

/// Every user's blob namespace, with the puts of blobs that existed already.
#[derive(Clone)]
pub struct BlobStores {
    objects: Arc<dyn ObjectStore>,
    again: Arc<PutAgain>,
}

impl BlobStores {
    /// The namespaces in `objects`, whose puts are timed by `clock`.
    pub fn new(objects: Arc<dyn ObjectStore>, clock: Arc<dyn Clock>) -> Self {
        Self {
            objects,
            again: Arc::new(PutAgain {
                clock,
                users: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// `user`'s namespace: the signed-in user's for uploads and downloads,
    /// the conversation owner's for transcript media.
    pub fn for_user(&self, user: &UserId) -> UserBlobs {
        UserBlobs {
            objects: self.objects.clone(),
            namespace: Path::from_iter(["blobs", user.as_str()]),
            clock: self.again.clock.clone(),
            puts: self.again.of(user),
        }
    }
}

/// Each user's puts of blobs that existed already, behind one lock per user.
/// It lives with the process: a restart forgets it, and with it only
/// references the stopped process had not written and never will.
struct PutAgain {
    clock: Arc<dyn Clock>,
    /// A `std` mutex: uploads at the edge and the shard threads share it, and
    /// each section only looks up or inserts a user's entry. An entry stays
    /// while the process runs, one per user who put a blob.
    users: Mutex<HashMap<UserId, Arc<UserPuts>>>,
}

/// When each of a user's blobs that existed already was last put again,
/// within the grace. Its lock is held across a put's question whether the
/// blob exists and the record of its answer, and across a collection's last
/// check of a blob and its deletion, so a put again comes either before
/// that check, which then keeps the blob, or after the deletion, and then
/// finds no blob and writes its bytes again. The lock is held for those
/// steps alone, never across a collection's listing or its reading of the
/// records.
type UserPuts = tokio::sync::Mutex<HashMap<BlobRef, Timestamp>>;

impl PutAgain {
    /// `user`'s puts, made on the first use.
    fn of(&self, user: &UserId) -> Arc<UserPuts> {
        // No section panics while it holds the lock, so a poisoned one still
        // holds a whole map.
        self.users
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .entry(user.clone())
            .or_default()
            .clone()
    }
}

/// Forgets the puts in `puts` past the grace at `now`.
fn forget_old(puts: &mut HashMap<BlobRef, Timestamp>, now: Timestamp) {
    let cutoff = now.as_millisecond() - GRACE_MS;
    puts.retain(|_, at| at.as_millisecond() >= cutoff);
}

/// One user's blobs.
#[derive(Clone)]
pub struct UserBlobs {
    objects: Arc<dyn ObjectStore>,
    namespace: Path,
    clock: Arc<dyn Clock>,
    puts: Arc<UserPuts>,
}

impl UserBlobs {
    /// Stores `bytes` and answers their name (`storage.md` § The object
    /// store). A name always names the same bytes, so a blob the namespace
    /// holds already is success: the put asks first whether it exists, and
    /// sends its bytes only when it does not, so repeated uploads of one file
    /// store it once and send it once.
    pub async fn put(&self, bytes: Bytes) -> Result<BlobRef, ObjectError> {
        let (bytes, blob) = with_name(bytes).await?;
        self.store(&blob, bytes).await?;
        Ok(blob)
    }

    /// [`UserBlobs::put`] of bytes the caller names by their SHA-256: false,
    /// and nothing stored, when they do not have it.
    pub async fn put_named(&self, bytes: Bytes, named: &BlobRef) -> Result<bool, ObjectError> {
        let (bytes, blob) = with_name(bytes).await?;
        if blob != *named {
            return Ok(false);
        }
        self.store(&blob, bytes).await?;
        Ok(true)
    }

    /// Stores `bytes` under `blob`, their name.
    async fn store(&self, blob: &BlobRef, bytes: Bytes) -> Result<(), ObjectError> {
        let location = self.location(blob);
        {
            let mut puts = self.puts.lock().await;
            match self.objects.head(&location).await {
                Ok(_) => {
                    self.put_again(&mut puts, blob);
                    return Ok(());
                }
                Err(object_store::Error::NotFound { .. }) => {}
                Err(error) => return Err(error.into()),
            }
        }
        let create = PutOptions {
            mode: PutMode::Create,
            ..PutOptions::default()
        };
        match self
            .objects
            .put_opts(&location, PutPayload::from_bytes(bytes), create)
            .await
        {
            Ok(_) => Ok(()),
            // A put of the same bytes created it since the HEAD.
            Err(object_store::Error::AlreadyExists { .. }) => {
                let mut puts = self.puts.lock().await;
                self.put_again(&mut puts, blob);
                Ok(())
            }
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

    /// The opening of `blob`, read as one range of the stored object, or
    /// `None` when this namespace does not hold it.
    pub async fn opening(&self, blob: &BlobRef) -> Result<Option<BlobOpening>, ObjectError> {
        let options = GetOptions::new().with_range(Some(0..OPENING_BYTES));
        match self.objects.get_opts(&self.location(blob), options).await {
            Ok(object) => {
                let size = object.meta.size;
                let bytes = object.bytes().await?;
                Ok(Some(BlobOpening { bytes, size }))
            }
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    /// Every blob the namespace holds, with when its object was written: one
    /// listing, which on S3 is one request per 1,000 objects. An object
    /// whose name is no blob's is left out: no record can name it, and a
    /// collection never deletes what it does not know.
    pub async fn list(&self) -> Result<Vec<StoredBlob>, ObjectError> {
        let mut listing = self.objects.list(Some(&self.namespace));
        let mut blobs = Vec::new();
        while let Some(object) = listing.next().await {
            let object = object?;
            let Some(name) = object.location.filename() else {
                continue;
            };
            let Ok(blob) = BlobRef::try_from(name.to_owned()) else {
                continue;
            };
            let written = Timestamp::from_millisecond(object.last_modified.timestamp_millis())
                .map_err(|error| ObjectError::Corrupt {
                    location: object.location.to_string(),
                    field: "last modified time",
                    reason: error.to_string(),
                })?;
            blobs.push(StoredBlob { blob, written });
        }
        Ok(blobs)
    }

    /// Records that `blob`, which the namespace held already, was put
    /// again now.
    fn put_again(&self, puts: &mut HashMap<BlobRef, Timestamp>, blob: &BlobRef) {
        let now = self.clock.now();
        forget_old(puts, now);
        puts.insert(blob.clone(), now);
    }

    /// A collection's last step for `blob`, which the listing found older
    /// than the grace and no record names: deletes it unless it was put
    /// again within the grace, and answers whether it did. Its age needs no
    /// second look: a put of a blob that exists writes nothing, and only a
    /// collection, one at a time per user, deletes one. A blob the namespace
    /// no longer holds is deleted already.
    pub async fn delete_unless_put_again(&self, blob: &BlobRef) -> Result<bool, ObjectError> {
        let mut puts = self.puts.lock().await;
        forget_old(&mut puts, self.clock.now());
        if puts.contains_key(blob) {
            return Ok(false);
        }
        match self.objects.delete(&self.location(blob)).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(true),
            Err(error) => Err(error.into()),
        }
    }

    fn location(&self, blob: &BlobRef) -> Path {
        self.namespace.clone().join(blob.as_str())
    }
}

/// `bytes` with their name. Hashing an upload of 25 MiB takes tens of
/// milliseconds, which would hold an async thread that long.
async fn with_name(bytes: Bytes) -> Result<(Bytes, BlobRef), ObjectError> {
    let named = tokio::task::spawn_blocking(move || {
        let blob = BlobRef::of(&bytes);
        (bytes, blob)
    })
    .await?;
    Ok(named)
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

    fn get<'a>(
        &'a self,
        blob: &'a BlobRef,
    ) -> LocalBoxFuture<'a, Result<Option<B64Bytes>, StoreError>> {
        Box::pin(async move {
            let bytes = UserBlobs::get(self, blob)
                .await
                .map_err(|error| StoreError::Failed(error.to_string()))?;
            Ok(bytes.map(B64Bytes::from))
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fmt;
    use std::sync::atomic::{AtomicBool, Ordering};

    use futures_util::FutureExt as _;
    use futures_util::stream::BoxStream;
    use object_store::memory::InMemory;
    use object_store::{
        CopyOptions, GetResult, ListResult, MultipartUpload, ObjectMeta, PutMultipartOptions,
        PutResult, RenameOptions, Result,
    };
    use tokio::sync::Notify;

    use super::*;

    /// An object store in memory whose answer that an object exists waits,
    /// once it has it and while `hold` is on, until the test lets it go.
    #[derive(Debug, Default)]
    struct HeldHeads {
        inner: InMemory,
        hold: AtomicBool,
        reached: Notify,
        go: Notify,
    }

    impl fmt::Display for HeldHeads {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            write!(formatter, "HeldHeads({})", self.inner)
        }
    }

    #[async_trait::async_trait]
    impl ObjectStore for HeldHeads {
        async fn put_opts(
            &self,
            location: &Path,
            payload: PutPayload,
            opts: PutOptions,
        ) -> Result<PutResult> {
            self.inner.put_opts(location, payload, opts).await
        }

        async fn put_multipart_opts(
            &self,
            location: &Path,
            opts: PutMultipartOptions,
        ) -> Result<Box<dyn MultipartUpload>> {
            self.inner.put_multipart_opts(location, opts).await
        }

        async fn get_opts(&self, location: &Path, options: GetOptions) -> Result<GetResult> {
            let head = options.head;
            let answer = self.inner.get_opts(location, options).await;
            if head && answer.is_ok() && self.hold.load(Ordering::SeqCst) {
                self.reached.notify_one();
                self.go.notified().await;
            }
            answer
        }

        fn delete_stream(
            &self,
            locations: BoxStream<'static, Result<Path>>,
        ) -> BoxStream<'static, Result<Path>> {
            self.inner.delete_stream(locations)
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
    async fn a_put_again_that_meets_a_collections_last_step_keeps_its_blob() {
        let objects = Arc::new(HeldHeads::default());
        let blobs = BlobStores::new(objects.clone(), Arc::new(demi_shared_types::SystemClock))
            .for_user(&UserId::try_from("ana").unwrap());
        let bytes = Bytes::from_static(b"a screenshot");
        let blob = blobs.put(bytes.clone()).await.unwrap();
        // A put of the same bytes learns that the blob exists, and is held
        // before it goes on.
        objects.hold.store(true, Ordering::SeqCst);
        let put = tokio::spawn({
            let (blobs, bytes) = (blobs.clone(), bytes.clone());
            async move { blobs.put(bytes).await }
        });
        objects.reached.notified().await;
        // The collection's last step for the blob, which nothing names, comes
        // now: it waits for the put rather than delete the blob under it.
        let mut deletion = std::pin::pin!(blobs.delete_unless_put_again(&blob));
        assert!(deletion.as_mut().now_or_never().is_none());
        objects.go.notify_one();
        assert_eq!(put.await.unwrap().unwrap(), blob);
        assert!(!deletion.await.unwrap(), "a blob put again within the day stays");
        assert_eq!(blobs.get(&blob).await.unwrap(), Some(bytes));
    }
}
