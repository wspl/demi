//! What an installation records beside its files about what it verified, so
//! a later process can trust the files without downloading them again.

use std::path::Path;

use serde::Serialize;

use crate::{Error, Mode, Permissions, Publication};

/// The receipt's file name inside an installation directory.
pub const FILE: &str = "receipt.json";

/// Durably records `receipt` in `directory`.
pub async fn write(directory: &Path, receipt: &impl Serialize) -> Result<(), Error> {
    let bytes = serde_json::to_vec(receipt).map_err(std::io::Error::other)?;
    crate::publish_bytes(
        &directory.join(FILE),
        &bytes,
        Publication {
            mode: Mode::Replace,
            permissions: Permissions::Default,
            durable: true,
        },
    )
    .await
}

/// The receipt's bytes, or none when `directory` has none. The caller decodes
/// them with its receipt type, which checks them.
pub async fn read(directory: &Path) -> Result<Option<Vec<u8>>, Error> {
    match tokio::fs::read(directory.join(FILE)).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}
