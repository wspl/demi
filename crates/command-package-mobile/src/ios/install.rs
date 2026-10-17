//! The iOS helper `demi-mobile` carries (`mobile.md` § iOS): a tar archive
//! compressed with zstd, holding `demi-ios-sim`, the libraries it links,
//! the guest bridge and idb's license notice, which the build embeds and
//! this unpacks on first use into `<installation>/mobile/<sha256>/`, checked
//! against the digest the build recorded.

use std::path::PathBuf;

#[cfg(demi_ios_helper)]
use tokio_util::sync::CancellationToken;

use crate::driver::{DriverError, Result};

/// The helper's file in the archive.
#[cfg(demi_ios_helper)]
const ENTRY: &str = "demi-ios-sim";

#[cfg(demi_ios_helper)]
const ARCHIVE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/demi-ios-sim.tar.zst"));
#[cfg(demi_ios_helper)]
const SHA256: &str = env!("DEMI_IOS_SIM_SHA256");

/// Whether this build carries the helper.
pub fn carried() -> bool {
    cfg!(demi_ios_helper)
}

/// The helper's executable, unpacked once into the installation.
pub async fn helper() -> Result<PathBuf> {
    #[cfg(not(demi_ios_helper))]
    {
        Err(DriverError::Failed("this build of demi-mobile carries no iOS helper".into()))
    }
    #[cfg(demi_ios_helper)]
    {
        let home = std::env::var_os("DEMI_HOME").ok_or_else(|| {
            DriverError::Failed("the service was started without DEMI_HOME, where the iOS helper goes".into())
        })?;
        let root = PathBuf::from(home).join("mobile");
        let archive = demi_shared_artifacts::Archive {
            digest: demi_shared_artifacts::Digest {
                size: ARCHIVE.len() as u64,
                sha256: SHA256.to_owned(),
            },
            entry: ENTRY.to_owned(),
        };
        let cancel = CancellationToken::new();
        let failed = |error: demi_shared_artifacts::Error| {
            DriverError::Failed(format!("the iOS helper could not be unpacked: {error}"))
        };
        match demi_shared_artifacts::install_archive(&root, &archive, &cancel)
            .await
            .map_err(failed)?
        {
            demi_shared_artifacts::ArchiveInstall::Installed(entry) => Ok(entry),
            demi_shared_artifacts::ArchiveInstall::Unpack(unpacking) => {
                tokio::fs::write(unpacking.archive(), ARCHIVE)
                    .await
                    .map_err(|error| DriverError::Failed(format!("the iOS helper could not be written: {error}")))?;
                unpacking.finish(&cancel).await.map_err(failed)
            }
        }
    }
}
