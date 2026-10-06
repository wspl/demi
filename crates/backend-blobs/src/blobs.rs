//! Attachment and transcript media (`storage.md` § Attachment and transcript
//! media, § The object store): each user's bytes under `blobs/<user>/`, named
//! by their SHA-256 in lowercase hexadecimal. A name identifies bytes only
//! within its user's namespace, so knowing another user's hash reaches
//! nothing of theirs. Nothing deletes a blob but the account's deletion
//! (`storage.md` § Retention).

use std::sync::Arc;

use bytes::Bytes;
use demi_agent_store::StoreError;
use demi_agent_store::media::BlobStore;
use demi_shared_types::{B64Bytes, BlobRef};
use demi_web_api_protocol::ids::UserId;
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

/// Every user's blob namespace.
#[derive(Clone)]
pub struct BlobStores {
    objects: Arc<dyn ObjectStore>,
}

impl BlobStores {
    /// The namespaces in `objects`.
    pub fn new(objects: Arc<dyn ObjectStore>) -> Self {
        Self { objects }
    }

    /// `user`'s namespace: the signed-in user's for uploads and downloads,
    /// the conversation owner's for transcript media.
    pub fn for_user(&self, user: &UserId) -> UserBlobs {
        UserBlobs {
            objects: self.objects.clone(),
            namespace: Path::from_iter(["blobs", user.as_str()]),
        }
    }
}

/// One user's blobs.
#[derive(Clone)]
pub struct UserBlobs {
    objects: Arc<dyn ObjectStore>,
    namespace: Path,
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
            Ok(_) => return Ok(()),
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
            // A put of the same bytes may have created it since the HEAD.
            Ok(_) | Err(object_store::Error::AlreadyExists { .. }) => Ok(()),
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
