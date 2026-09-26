//! The verified executable cache (`native-runtime.md` § Install the selected
//! executable): a miss downloads and verifies, a hit asks nobody and reads
//! nothing, and nothing partial or mismatched is ever published.

use demi_command_service::protocol::PackageArtifact;
use demi_runner::services::{ArtifactResolver, ArtifactSource, RuntimeError, cache::ArtifactCache};
use futures_util::future::BoxFuture;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio_util::sync::CancellationToken;

struct Resolver {
    path: PathBuf,
    calls: AtomicUsize,
}

impl ArtifactResolver for Resolver {
    fn resolve<'a>(
        &'a self,
        _: &'a PackageArtifact,
        _: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok(ArtifactSource::Local(self.path.clone()))
        })
    }
}

fn artifact(bytes: &[u8]) -> PackageArtifact {
    PackageArtifact {
        sha256: format!("{:x}", Sha256::digest(bytes)),
        size: bytes.len() as u64,
    }
}

/// An entry was verified as it was published, so a hit reuses it unread:
/// a service start does not hash the whole executable again.
#[tokio::test]
async fn a_hit_asks_nobody_reads_nothing_and_an_entry_of_another_size_fails() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let bytes = b"native executable fixture";
    tokio::fs::write(&source, bytes).await.unwrap();
    let resolver = Resolver {
        path: source,
        calls: AtomicUsize::new(0),
    };
    let cache = ArtifactCache::new(root.path().join("cache")).await.unwrap();
    let cancel = CancellationToken::new();
    let path = cache.install(&artifact(bytes), &resolver, &cancel).await.unwrap();
    assert_eq!(tokio::fs::read(&path).await.unwrap(), bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "the cached executable runs");
    }
    cache.install(&artifact(bytes), &resolver, &cancel).await.unwrap();
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    // Other bytes of the declared size: a hit that read the entry would
    // refuse them.
    tokio::fs::write(&path, vec![b'x'; bytes.len()]).await.unwrap();
    assert_eq!(cache.install(&artifact(bytes), &resolver, &cancel).await.unwrap(), path);
    tokio::fs::write(&path, b"corrupt cache").await.unwrap();
    let result = cache.install(&artifact(bytes), &resolver, &cancel).await;
    assert!(
        matches!(result, Err(RuntimeError::Artifact(demi_artifact::Error::Size { .. }))),
        "{result:?}"
    );
}

#[tokio::test]
async fn a_mismatched_download_leaves_no_file_behind() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    tokio::fs::write(&source, b"too much data").await.unwrap();
    let resolver = Resolver {
        path: source,
        calls: AtomicUsize::new(0),
    };
    let cache_root = root.path().join("cache");
    let cache = ArtifactCache::new(cache_root.clone()).await.unwrap();
    let result = cache
        .install(&artifact(b"small"), &resolver, &CancellationToken::new())
        .await;
    assert!(matches!(result, Err(RuntimeError::Artifact(_))), "{result:?}");
    assert!(
        tokio::fs::read_dir(cache_root)
            .await
            .unwrap()
            .next_entry()
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn a_cancelled_install_publishes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    tokio::fs::write(&source, b"bytes").await.unwrap();
    let resolver = Resolver {
        path: source,
        calls: AtomicUsize::new(0),
    };
    let cache_root = root.path().join("cache");
    let cache = ArtifactCache::new(cache_root.clone()).await.unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let result = cache.install(&artifact(b"bytes"), &resolver, &cancel).await;
    assert!(matches!(result, Err(RuntimeError::Cancelled)), "{result:?}");
    assert!(
        tokio::fs::read_dir(cache_root)
            .await
            .unwrap()
            .next_entry()
            .await
            .unwrap()
            .is_none()
    );
}
