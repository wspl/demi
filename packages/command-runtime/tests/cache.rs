use demi_command_protocol::PackageArtifact;
use demi_command_runtime::{ArtifactCache, ArtifactResolver, ArtifactSource, RuntimeError};
use futures_util::future::BoxFuture;
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

struct Resolver {
    path: PathBuf,
    calls: AtomicUsize,
    started: Notify,
    proceed: Notify,
}

impl ArtifactResolver for Resolver {
    fn resolve<'a>(
        &'a self,
        _: &'a PackageArtifact,
        _: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.started.notify_one();
            self.proceed.notified().await;
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

#[tokio::test]
async fn cancelled_waiter_does_not_cancel_shared_download_and_cache_is_verified() {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("source");
        let bytes = b"native executable fixture";
        tokio::fs::write(&source, bytes).await.unwrap();
        let resolver = Arc::new(Resolver {
            path: source,
            calls: AtomicUsize::new(0),
            started: Notify::new(),
            proceed: Notify::new(),
        });
        let cache = ArtifactCache::new(root.path().join("cache")).await.unwrap();
        let cancel = CancellationToken::new();
        let first = tokio::spawn({
            let cache = cache.clone();
            let resolver = resolver.clone();
            let cancel = cancel.clone();
            async move { cache.acquire(artifact(bytes), resolver, &cancel).await }
        });
        resolver.started.notified().await;
        cancel.cancel();
        assert!(matches!(
            first.await.unwrap().unwrap_err().as_ref(),
            RuntimeError::Cancelled
        ));
        resolver.proceed.notify_one();
        let path = cache
            .acquire(artifact(bytes), resolver.clone(), &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(tokio::fs::read(&path).await.unwrap(), bytes);
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
        cache
            .acquire(artifact(bytes), resolver.clone(), &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
        tokio::fs::write(&path, b"corrupt cache").await.unwrap();
        assert!(
            cache
                .acquire(artifact(bytes), resolver.clone(), &CancellationToken::new())
                .await
                .is_err()
        );
        cache.shutdown().await;
        assert!(
            cache
                .acquire(artifact(bytes), resolver, &CancellationToken::new())
                .await
                .is_err()
        );
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn size_mismatch_leaves_no_cache_file_or_temporary_download() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    tokio::fs::write(&source, b"too much data").await.unwrap();
    let resolver = Arc::new(Resolver {
        path: source,
        calls: AtomicUsize::new(0),
        started: Notify::new(),
        proceed: Notify::new(),
    });
    resolver.proceed.notify_one();
    let cache_root = root.path().join("cache");
    let cache = ArtifactCache::new(cache_root.clone()).await.unwrap();
    assert!(
        cache
            .acquire(artifact(b"small"), resolver, &CancellationToken::new())
            .await
            .is_err()
    );
    cache.shutdown().await;
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
