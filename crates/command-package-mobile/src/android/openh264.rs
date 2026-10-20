//! The H.264 encoder of an emulator's live view: Cisco's own build of OpenH264,
//! which Cisco pays the patent royalties for, as Firefox uses it.
//!
//! Cisco pays for its binary only, not for a build Demi would make and ship
//! from the source, so Demi does not ship it. The first view of an emulator
//! downloads it from Cisco and checks it against its pinned hash before the
//! program loads it, and keeps it for the next.
//!
//! It encodes every emulator's pictures, on a Mac too, rather than using
//! VideoToolbox. The iOS helper's encoder lives inside the helper, which needs
//! Xcode, so a Mac with only Android Studio could not use it. A second
//! VideoToolbox encoder for Android alone would be the same thing written
//! twice.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

use futures_util::future::BoxFuture;
use openh264::OpenH264API;

use crate::driver::{DriverError, Result};

/// Makes an encoder's interface to the library, once for each encoder.
pub type Load = Arc<dyn Fn() -> Result<OpenH264API> + Send + Sync>;

/// Where the encoder's library comes from; a test gives its own.
pub trait Codec: Send + Sync + 'static {
    /// The library, fetched first when this Host has none.
    fn library(&self) -> BoxFuture<'_, Result<Load>>;
}

/// Cisco's build for this system: its file on Cisco's server, compressed
/// with bzip2 there, and the SHA-256 of the library itself.
struct Binary {
    file: &'static str,
    sha256: &'static str,
}

/// Cisco's OpenH264 2.6.0, the version the `openh264` crate's interface
/// is generated for; the hashes are the ones that crate lists.
const BINARY: Option<Binary> = if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
    Some(Binary {
        file: "libopenh264-2.6.0-mac-arm64.dylib",
        sha256: "052e98bfcf7a9167d22f3bbb3f5988ef79065591f36af8b52924b22b13624551",
    })
} else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
    Some(Binary {
        file: "libopenh264-2.6.0-mac-x64.dylib",
        sha256: "e3dc8bc01fe69363f61fd3c02fd27798537a585eadd38cd808f303d1ee505a19",
    })
} else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
    Some(Binary {
        file: "libopenh264-2.6.0-linux64.8.so",
        sha256: "2f0cde7c6a6abcf5cae76942894ea42897fa677bce4ed6c91a24dd1b041d5f04",
    })
} else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
    Some(Binary {
        file: "libopenh264-2.6.0-linux-arm64.8.so",
        sha256: "12e7b33623667cdab0e575170c147b1b36eadb77d0d2aa7ceb5afd3e58902140",
    })
} else if cfg!(all(windows, target_arch = "x86_64")) {
    Some(Binary {
        file: "openh264-2.6.0-win64.dll",
        sha256: "2076cb5675ec6c1a4c70e7a2a322552f547b6eeed649d6dfcd9e02a543b24691",
    })
} else if cfg!(all(windows, target_arch = "aarch64")) {
    Some(Binary {
        file: "openh264-2.6.0-win-arm64.dll",
        sha256: "fb75103938f4f47d119b983e06334df41a803bc72fb5c46e3623f6fea5782732",
    })
} else {
    None
};

/// Where Cisco serves its builds.
const CISCO: &str = "https://ciscobinary.openh264.org";

/// Cisco's build, kept in `directory`.
pub struct Cisco {
    directory: PathBuf,
    origin: String,
    /// The checked library, fetched once.
    fetched: tokio::sync::OnceCell<PathBuf>,
}

impl Cisco {
    pub fn new(directory: PathBuf) -> Self {
        Self::from_origin(directory, CISCO.to_owned())
    }

    /// Cisco's build as `origin` serves it, such as a test's server.
    pub fn from_origin(directory: PathBuf, origin: String) -> Self {
        Self {
            directory,
            origin,
            fetched: tokio::sync::OnceCell::new(),
        }
    }

    /// The library's path, downloaded when this Host has none or one that
    /// is not Cisco's.
    async fn fetch(&self) -> Result<PathBuf> {
        let binary = BINARY.as_ref().ok_or_else(|| {
            DriverError::Failed("Cisco builds no OpenH264 for this system, so an emulator cannot be shown".into())
        })?;
        let path = self.directory.join(binary.file);
        if let Ok(kept) = tokio::fs::read(&path).await
            && sha256(&kept) == binary.sha256
        {
            return Ok(path);
        }
        let url = format!("{}/{}.bz2", self.origin, binary.file);
        let failed = |error: &dyn std::fmt::Display| {
            DriverError::Failed(format!("Cisco's OpenH264 did not download from {url}: {error}"))
        };
        // The library is checked against its pinned hash once unpacked, so
        // neither plain HTTP nor a redirect can change what is kept.
        let client = demi_shared_artifacts::download_client_allowing_http().map_err(|error| failed(&error))?;
        let request = demi_shared_artifacts::Download {
            redirects: 10,
            ..demi_shared_artifacts::Download::new(&url)
        };
        // Nothing cancels the fetch but the call that awaits it going away.
        let (received, compressed) =
            demi_shared_artifacts::download_bytes(&client, request, &tokio_util::sync::CancellationToken::new())
                .await
                .map_err(|error| failed(&error))?;
        received.plain().map_err(|error| failed(&error))?;
        let library = tokio::task::spawn_blocking(move || {
            let mut library = Vec::new();
            std::io::Read::read_to_end(&mut bzip2::read::BzDecoder::new(compressed.as_slice()), &mut library)
                .map(|_| library)
        })
        .await
        .map_err(|error| failed(&error))?
        .map_err(|error| failed(&error))?;
        if sha256(&library) != binary.sha256 {
            return Err(DriverError::Failed(format!(
                "Cisco's OpenH264 from {url} does not match its pinned hash, so it is not used"
            )));
        }
        keep(&self.directory, &path, &library)
            .await
            .map_err(|error| DriverError::Failed(format!("Cisco's OpenH264 was not kept in {}: {error}", path.display())))?;
        Ok(path)
    }
}

/// Writes `library` to `path` through a file beside it, so a program
/// killed mid-write leaves no library half written.
async fn keep(directory: &Path, path: &Path, library: &[u8]) -> std::io::Result<()> {
    tokio::fs::create_dir_all(directory).await?;
    let partial = path.with_extension("partial");
    tokio::fs::write(&partial, library).await?;
    tokio::fs::rename(&partial, path).await
}

fn sha256(bytes: &[u8]) -> String {
    aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, bytes)
        .as_ref()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

impl Codec for Cisco {
    fn library(&self) -> BoxFuture<'_, Result<Load>> {
        Box::pin(async move {
            let path = self.fetched.get_or_try_init(|| self.fetch()).await?.clone();
            // The crate checks the library against Cisco's hashes again as
            // it loads it, so a file changed since is refused.
            Ok(Arc::new(move || {
                OpenH264API::from_blob_path(&path)
                    .map_err(|error| DriverError::Failed(format!("Cisco's OpenH264 did not load: {error}")))
            }) as Load)
        })
    }
}
