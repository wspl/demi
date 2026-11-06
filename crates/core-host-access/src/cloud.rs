//! The Cloud as host access sees it: a Host that whoever runs the core wakes
//! and stops, whose admission an operation on it holds.

use demi_core_store::StorageError;
use demi_core_store::devices::DeviceRecord;
use demi_runner_client::Admission;
use demi_shared_gates::GateLease;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::DeviceId;
use futures_util::future::LocalBoxFuture;

/// What host access asks of the Cloud.
pub trait CloudHost {
    /// The Cloud's device, made on its first use.
    fn cloud_device(&self) -> LocalBoxFuture<'_, Result<DeviceRecord, CloudError>>;

    /// Takes the Cloud's admission: it wakes a stopped Cloud, joins a boot
    /// under way, or waits for a running reset to finish.
    fn admit_cloud<'a>(&'a self, device: &'a DeviceRecord) -> LocalBoxFuture<'a, Result<CloudAdmission, CloudError>>;

    /// What each operation on the Cloud `device` takes, for one that holds no
    /// admission of its own, as a command its runner took up.
    fn operation_admission<'a>(&'a self, device: &'a DeviceRecord) -> LocalBoxFuture<'a, Result<Admission, StorageError>>;
}

/// The Cloud of `shard`'s starter.
pub fn cloud_of(shard: &(impl crate::HostShard + ?Sized)) -> Result<std::rc::Rc<dyn CloudHost>, CloudError> {
    shard.cloud_host().ok_or(CloudError::Absent)
}

/// Why the Cloud admits no operation, or why one of its transitions failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CloudError {
    /// Whoever runs the core runs no Cloud.
    #[error("No Cloud runs here")]
    Absent,
    #[error("Cloud is shutting down")]
    Closed,
    /// Its runtime was lost too often in a short time.
    #[error("Cloud repeatedly failed; reset the environment to recover")]
    CrashLoop,
    #[error("Cloud capacity is currently full; retry later")]
    Capacity,
    /// Another reset holds the Cloud.
    #[error("Another reset is in progress")]
    Resetting,
    /// A boot, a save or a reset failed, or the machine manager did.
    #[error("{0}")]
    Failed(String),
}

impl CloudError {
    /// The code and HTTP status an operation refused this way answers with.
    pub fn code(&self) -> (ErrorCode, u16) {
        match self {
            Self::Absent => (ErrorCode::CloudUnavailable, 503),
            Self::Closed => (ErrorCode::BackendClosing, 503),
            Self::CrashLoop => (ErrorCode::CloudCrashLoop, 503),
            Self::Capacity => (ErrorCode::CloudCapacity, 503),
            Self::Resetting => (ErrorCode::CloudResetting, 409),
            Self::Failed(_) => (ErrorCode::CloudUnavailable, 503),
        }
    }
}

impl From<StorageError> for CloudError {
    fn from(error: StorageError) -> Self {
        tracing::error!(
            event = "cloud_records.failed",
            error = &error as &dyn std::error::Error,
            "the Cloud's records failed"
        );
        Self::Failed(format!(
            "The Cloud's records could not be read or written: {error}"
        ))
    }
}

/// A hold of the Cloud's admission: the Cloud keeps running while it is
/// held, and each operation of a Host made under it takes `per_operation`.
pub struct CloudAdmission {
    pub device: DeviceId,
    pub per_operation: Admission,
    _held: GateLease,
}

impl CloudAdmission {
    pub fn new(device: DeviceId, per_operation: Admission, held: GateLease) -> Self {
        Self {
            device,
            per_operation,
            _held: held,
        }
    }
}
