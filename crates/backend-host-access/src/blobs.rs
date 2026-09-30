//! A user's blobs as the commits of the user's conversation databases reach
//! them: the tree store's checkpoints, and the records of the commands'
//! outputs that the nodes' shells keep (`storage.md` § Collecting blobs).

use demi_agent_store::StoreError;
use demi_agent_store::media::BlobStore;
use demi_backend_objects::blobs::UserBlobs;
use demi_backend_storage::blob_refs::OwnerBlobs;
use demi_core::BlobRef;

/// The namespace and its record of blob uses, which the object store keeps
/// and storage reads through `OwnerBlobs`.
#[derive(Clone)]
pub struct ConversationBlobs(pub UserBlobs);

impl OwnerBlobs for ConversationBlobs {
    fn media(&self) -> &dyn BlobStore {
        &self.0
    }

    fn commit_uses(&self, blobs: &[BlobRef]) -> Result<(), StoreError> {
        self.0.commit_uses(blobs)
    }
}
