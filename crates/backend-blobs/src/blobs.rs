//! Attachment and transcript media (`storage.md` § Attachment and transcript
//! media, § The object store): each user's bytes under `blobs/<user>/`, named
//! by their SHA-256 in lowercase hexadecimal. A name identifies bytes only
//! within its user's namespace, so knowing another user's hash reaches
//! nothing of theirs. A collection of the namespace lists it and deletes
//! the blobs nothing names (`storage.md` § Deleting a conversation); every
//! put of a blob the namespace holds already is remembered for a day, since
//! it writes nothing that would show its age.

use std::collections::{HashMap, HashSet};
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
                puts: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// `user`'s namespace: the signed-in user's for uploads and downloads,
    /// the conversation owner's for transcript media.
    pub fn for_user(&self, user: &UserId) -> UserBlobs {
        UserBlobs {
            objects: self.objects.clone(),
            namespace: Path::from_iter(["blobs", user.as_str()]),
            user: user.clone(),
            again: self.again.clone(),
        }
    }
}

/// When each user's blobs that existed already were last put again, within
/// the grace. It lives with the process: a restart forgets it, and with it
/// only references the stopped process had not written and never will.
struct PutAgain {
    clock: Arc<dyn Clock>,
    /// A `std` mutex: uploads at the edge and the shard threads share it,
    /// and each section only reads or changes the map.
    puts: Mutex<HashMap<UserId, HashMap<BlobRef, Timestamp>>>,
}

impl PutAgain {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<UserId, HashMap<BlobRef, Timestamp>>> {
        // No section panics while it holds the lock, so a poisoned one still
        // holds a whole map.
        self.puts.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The time, in milliseconds, before which a put at `now` is past the
    /// grace.
    fn cutoff(now: Timestamp) -> i64 {
        now.as_millisecond() - GRACE_MS
    }

    fn record(&self, user: &UserId, blob: &BlobRef) {
        let now = self.clock.now();
        let cutoff = Self::cutoff(now);
        let mut puts = self.lock();
        let user_puts = puts.entry(user.clone()).or_default();
        user_puts.retain(|_, at| at.as_millisecond() >= cutoff);
        user_puts.insert(blob.clone(), now);
    }

    fn recent(&self, user: &UserId) -> HashSet<BlobRef> {
        let cutoff = Self::cutoff(self.clock.now());
        let mut puts = self.lock();
        let Some(user_puts) = puts.get_mut(user) else {
            return HashSet::new();
        };
        user_puts.retain(|_, at| at.as_millisecond() >= cutoff);
        let recent = user_puts.keys().cloned().collect();
        if user_puts.is_empty() {
            puts.remove(user);
        }
        recent
    }
}

/// One user's blobs.
#[derive(Clone)]
pub struct UserBlobs {
    objects: Arc<dyn ObjectStore>,
    namespace: Path,
    user: UserId,
    again: Arc<PutAgain>,
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
        match self.objects.head(&location).await {
            Ok(_) => {
                self.again.record(&self.user, blob);
                return Ok(());
            }
            Err(object_store::Error::NotFound { .. }) => {}
            Err(error) => return Err(error.into()),
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
                self.again.record(&self.user, blob);
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

    /// The blobs put again within the last [`GRACE_MS`]: each a put of a
    /// blob the namespace held already, which wrote nothing. Older ones are
    /// forgotten.
    pub fn put_again_recently(&self) -> HashSet<BlobRef> {
        self.again.recent(&self.user)
    }

    /// Deletes `blob`; one the namespace does not hold is deleted already.
    pub async fn delete(&self, blob: &BlobRef) -> Result<(), ObjectError> {
        match self.objects.delete(&self.location(blob)).await {
            Ok(()) | Err(object_store::Error::NotFound { .. }) => Ok(()),
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
