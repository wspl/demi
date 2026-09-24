//! Attachment and transcript media (`storage.md` § Attachment and transcript
//! media, § The object store): each user's bytes under `blobs/<user>/`, named
//! by their SHA-256 in lowercase hexadecimal. A name identifies bytes only
//! within its user's namespace, so knowing another user's hash reaches
//! nothing of theirs.

use std::sync::Arc;

use bytes::Bytes;
use demi_agent::store::StoreError;
use demi_agent::store::media::BlobStore;
use demi_core::{B64Bytes, BlobRef};
use demi_web_api::ids::UserId;
use futures_util::future::LocalBoxFuture;
use object_store::path::Path;
use object_store::{ObjectStore, ObjectStoreExt as _, PutMode, PutOptions, PutPayload};
use sha2::{Digest, Sha256};

use super::StorageError;

/// Every user's blob namespace.
#[derive(Clone)]
pub(crate) struct BlobStores {
    objects: Arc<dyn ObjectStore>,
}

impl BlobStores {
    pub(crate) fn new(objects: Arc<dyn ObjectStore>) -> Self {
        Self { objects }
    }

    /// `user`'s namespace: the signed-in user's for uploads and downloads,
    /// the conversation owner's for transcript media.
    pub(crate) fn for_user(&self, user: &UserId) -> UserBlobs {
        UserBlobs {
            objects: self.objects.clone(),
            namespace: Path::from_iter(["blobs", user.as_str()]),
        }
    }
}

/// One user's blobs.
#[derive(Clone)]
pub(crate) struct UserBlobs {
    objects: Arc<dyn ObjectStore>,
    namespace: Path,
}

impl UserBlobs {
    /// Stores `bytes` and answers their name. A name always names the same
    /// bytes, so finding the object there already is success: repeated
    /// uploads of one file store it once.
    pub(crate) async fn put(&self, bytes: Bytes) -> Result<BlobRef, StorageError> {
        // Hashing an upload of 25 MiB takes tens of milliseconds, which would
        // hold an async thread that long.
        let (bytes, digest) = tokio::task::spawn_blocking(move || {
            let digest = hex::encode(Sha256::digest(&bytes));
            (bytes, digest)
        })
        .await?;
        let blob = BlobRef::try_from(digest).expect("a SHA-256 in lowercase hexadecimal names a blob");
        let create = PutOptions {
            mode: PutMode::Create,
            ..PutOptions::default()
        };
        match self.objects.put_opts(&self.location(&blob), PutPayload::from_bytes(bytes), create).await {
            Ok(_) | Err(object_store::Error::AlreadyExists { .. }) => Ok(blob),
            Err(error) => Err(error.into()),
        }
    }

    /// The bytes `blob` names, or `None` when this namespace does not hold
    /// them.
    pub(crate) async fn get(&self, blob: &BlobRef) -> Result<Option<Bytes>, StorageError> {
        match self.objects.get(&self.location(blob)).await {
            Ok(object) => Ok(Some(object.bytes().await?)),
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(error) => Err(error.into()),
        }
    }

    fn location(&self, blob: &BlobRef) -> Path {
        self.namespace.clone().join(blob.as_str())
    }
}

/// The namespace as the agent's media mapping reaches it: the conversation
/// owner's, where a tree store keeps a block's media and the socket the
/// media of the frames it sends (`runtime.md` § Media).
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::objects;

    fn user(id: &str) -> UserId {
        UserId::try_from(id).unwrap()
    }

    #[tokio::test]
    async fn blobs_are_named_by_their_bytes_stored_once_and_kept_per_user() {
        let data = tempfile::tempdir().unwrap();
        let blobs = BlobStores::new(objects::open(data.path()).await.unwrap());
        let ana = blobs.for_user(&user("ana"));

        let name = ana.put(Bytes::from_static(b"\x0a\x14\x1e")).await.unwrap();
        assert_eq!(name.as_str(), hex::encode(Sha256::digest(b"\x0a\x14\x1e")));
        assert_eq!(ana.put(Bytes::from_static(b"\x0a\x14\x1e")).await.unwrap(), name);
        assert_eq!(ana.get(&name).await.unwrap().unwrap(), Bytes::from_static(b"\x0a\x14\x1e"));
        let file = data.path().join("blobs").join("ana").join(name.as_str());
        assert_eq!(std::fs::read(file).unwrap(), b"\x0a\x14\x1e");

        let missing = BlobRef::try_from("0".repeat(64)).unwrap();
        assert_eq!(ana.get(&missing).await.unwrap(), None);
        // Another user's namespace does not hold ana's bytes, whatever the name.
        assert_eq!(blobs.for_user(&user("ben")).get(&name).await.unwrap(), None);
    }
}
