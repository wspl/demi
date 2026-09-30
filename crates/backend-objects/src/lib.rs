//! The object store (`storage.md` § The object store), on local disk or S3
//! through `object_store`, and the attachment and transcript media over it
//! with the record of each blob's uses (`storage.md` § Collecting blobs).

pub mod blobs;
#[cfg(any(test, feature = "testing"))]
pub mod counting;
#[cfg(any(test, feature = "testing"))]
pub mod fake_s3;
pub mod store;

/// Why an operation on the object store failed.
#[derive(Debug, thiserror::Error)]
pub enum ObjectError {
    #[error("the object store failed: {0}")]
    Store(#[from] object_store::Error),
    /// An object's metadata outside its type, such as a write time out of
    /// range; nothing repairs it.
    #[error("the object {location} holds an invalid {field}: {reason}")]
    Corrupt {
        location: String,
        field: &'static str,
        reason: String,
    },
    /// Work handed to a blocking thread ended without an answer, because it
    /// panicked or the runtime is shutting down.
    #[error("blocking object store work did not finish: {0}")]
    Interrupted(#[from] tokio::task::JoinError),
}

