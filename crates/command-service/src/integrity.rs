//! Bounded hashing shared by native package and browser installation verification.

use sha2::{Digest, Sha256};
use tokio::io::AsyncReadExt;

use crate::{ServiceError, protocol::PackageArtifact};

/// Hash a native artifact without retaining its bytes; reject growth beyond its limit.
pub async fn artifact_digest(
    path: &std::path::Path,
    limit: u64,
    cancel: &tokio_util::sync::CancellationToken,
) -> Result<PackageArtifact, ServiceError> {
    let mut file = tokio::fs::File::open(path).await?;
    let mut hash = Sha256::new();
    let mut size = 0_u64;
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let count = tokio::select! {
            _ = cancel.cancelled() => return Err(ServiceError::Cancelled),
            read = file.read(&mut buffer) => read?,
        };
        if count == 0 {
            break;
        }
        size = size
            .checked_add(count as u64)
            .filter(|size| *size <= limit)
            .ok_or_else(|| {
                ServiceError::Handler("artifact exceeds its declared size limit".into())
            })?;
        hash.update(&buffer[..count]);
    }
    Ok(PackageArtifact {
        size,
        sha256: format!("{:x}", hash.finalize()),
    })
}
