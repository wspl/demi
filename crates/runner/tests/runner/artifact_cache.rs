//! The verified executable cache (`native-runtime.md` § Install the selected
//! executable): a miss downloads and verifies, a hit asks nobody and reads
//! nothing, and nothing partial or mismatched is ever published.

use demi_command_service::protocol::PackageArtifact;
use demi_runner::host_log::{self, Query};
use demi_runner::services::{ArtifactResolver, ArtifactSource, RuntimeError, cache::ArtifactCache};
use futures_util::future::BoxFuture;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio_util::sync::CancellationToken;
use tracing::instrument::WithSubscriber as _;
use tracing_subscriber::layer::SubscriberExt as _;

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
    let cache = ArtifactCache::new(root.path().join("cache"), None).await.unwrap();
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
    let cache = ArtifactCache::new(cache_root.clone(), None).await.unwrap();
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
    let cache = ArtifactCache::new(cache_root.clone(), None).await.unwrap();
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

/// A resolver that serves `bytes` from a file in `root`, as the backend
/// serves the executable they make.
async fn serving(root: &Path, bytes: &[u8]) -> Resolver {
    let path = root.join(format!("{}.source", artifact(bytes).sha256));
    tokio::fs::write(&path, bytes).await.unwrap();
    Resolver {
        path,
        calls: AtomicUsize::new(0),
    }
}

/// Writes `bytes` where a Cloud image rooted at `image` preinstalls the
/// executable of `artifact` (`native-runtime.md` § Preinstalled executables)
/// and returns its path.
async fn preinstall(image: &Path, artifact: &PackageArtifact, bytes: &[u8]) -> PathBuf {
    let directory = image.join(&artifact.sha256);
    tokio::fs::create_dir_all(&directory).await.unwrap();
    let executable = directory.join("demi-commands");
    tokio::fs::write(&executable, bytes).await.unwrap();
    executable
}

/// The image's copy of an executable that matches its descriptor is what
/// every start runs: the backend is not asked, and nothing is copied into
/// the cache.
#[tokio::test]
async fn a_matching_preinstalled_executable_is_used_without_a_download() {
    let root = tempfile::tempdir().unwrap();
    let bytes = b"native executable fixture";
    let image = root.path().join("image");
    let copy = preinstall(&image, &artifact(bytes), bytes).await;
    let resolver = serving(root.path(), bytes).await;
    let cache_root = root.path().join("cache");
    let cache = ArtifactCache::new(cache_root.clone(), Some(image))
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    // Two starts, as of a service that stopped and is needed again.
    for _ in 0..2 {
        let path = cache
            .install(&artifact(bytes), &resolver, &cancel)
            .await
            .unwrap();
        assert_eq!(path, copy);
    }
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
    let mut cached = tokio::fs::read_dir(cache_root).await.unwrap();
    assert!(cached.next_entry().await.unwrap().is_none());
}

/// Without a copy that matches, the runner downloads as it would without an
/// image. A copy that does not match is passed over and the Host log says
/// why, once; an executable the image holds no copy of, as on every paired
/// device, is downloaded without a word.
#[tokio::test]
async fn without_a_matching_preinstalled_executable_the_download_happens() {
    let root = tempfile::tempdir().unwrap();
    let image = root.path().join("image");
    let damaged = b"native executable fixture";
    preinstall(&image, &artifact(damaged), &vec![b'x'; damaged.len()]).await;
    let absent = b"an executable the image lacks";
    let damaged_source = serving(root.path(), damaged).await;
    let absent_source = serving(root.path(), absent).await;
    let cache = ArtifactCache::new(root.path().join("cache"), Some(image.clone()))
        .await
        .unwrap();
    // As `main` makes it, the log is a layer of the subscriber.
    let (log, layer) = host_log::open(root.path().join("log")).await.unwrap();
    let subscriber = tracing_subscriber::registry().with(layer);
    let cancel = CancellationToken::new();
    let executables = [
        (&damaged[..], &damaged_source),
        (&absent[..], &absent_source),
    ];
    async {
        // Two starts of each, as of a service that stopped and is needed
        // again.
        for _ in 0..2 {
            for (bytes, resolver) in executables {
                let path = cache
                    .install(&artifact(bytes), resolver, &cancel)
                    .await
                    .unwrap();
                assert_eq!(tokio::fs::read(&path).await.unwrap(), bytes);
            }
        }
    }
    .with_subscriber(subscriber)
    .await;
    assert_eq!(damaged_source.calls.load(Ordering::SeqCst), 1);
    assert_eq!(absent_source.calls.load(Ordering::SeqCst), 1);
    let query = Query {
        since: None,
        limit: 10,
        source: None,
    };
    let lines = log.reader().read(query).await.unwrap().lines;
    log.close().await;
    let [line] = lines.as_slice() else {
        panic!("one line for the damaged copy: {lines:?}");
    };
    let directory = image.join(artifact(damaged).sha256);
    assert_eq!(line.source, "runner");
    assert!(
        line.text.contains(&*directory.to_string_lossy())
            && line.text.contains("does not match its declared SHA-256"),
        "{}",
        line.text
    );
}
