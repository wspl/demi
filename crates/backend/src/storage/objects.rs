//! The object store blobs and the change store share (`storage.md` § The
//! object store): the data directory of a single-backend deployment, reached
//! through `object_store`, so a blob is the file `blobs/<user>/<sha256>` in
//! it and the change store's objects are under `changes/`.

use std::path::Path;
use std::sync::Arc;

use object_store::ObjectStore;
use object_store::local::LocalFileSystem;

use super::StorageError;

pub(crate) async fn open(data_dir: &Path) -> Result<Arc<dyn ObjectStore>, StorageError> {
    let root = data_dir.to_owned();
    // Resolving the directory is file system work.
    let local = tokio::task::spawn_blocking(move || LocalFileSystem::new_with_prefix(root)).await??;
    Ok(Arc::new(local))
}
