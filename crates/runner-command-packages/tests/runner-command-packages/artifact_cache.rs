//! The verified artifact cache (`native-runtime.md` § Install the selected
//! package): a miss downloads and verifies, a hit asks nobody, nothing
//! partial or mismatched is ever published, and a download is an install
//! the runner reports until it ends (§ Installation progress).

use demi_command_protocol::{PackageArtifact, ResourceArtifact};
use demi_runner_command_packages::{
    ArtifactResolver, ArtifactSource, Installs, RuntimeError,
    cache::{ArtifactCache, ForPackage},
};
use demi_runner_protocol::wire::{Install, InstallArtifact, InstallPhase};
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

/// The package the tests install for.
fn package(resolver: &Resolver) -> ForPackage<'_> {
    ForPackage {
        id: "demi.fixture",
        resolver,
    }
}

fn artifact(bytes: &[u8]) -> PackageArtifact {
    PackageArtifact {
        sha256: format!("{:x}", Sha256::digest(bytes)),
        size: bytes.len() as u64,
    }
}

/// An entry was verified as it was published, so a second install reuses it
/// without asking the backend; an entry of another size is not the one
/// declared.
#[tokio::test]
async fn a_cached_executable_is_reused_without_asking_and_one_of_another_size_fails() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let bytes = b"native executable fixture";
    tokio::fs::write(&source, bytes).await.unwrap();
    let resolver = Resolver {
        path: source,
        calls: AtomicUsize::new(0),
    };
    let cache = ArtifactCache::new(root.path().join("cache"), None, Installs::default())
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    let path = cache
        .install(&artifact(bytes), package(&resolver), &cancel)
        .await
        .unwrap();
    assert_eq!(tokio::fs::read(&path).await.unwrap(), bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "the cached executable runs");
    }
    assert_eq!(
        cache
            .install(&artifact(bytes), package(&resolver), &cancel)
            .await
            .unwrap(),
        path
    );
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    tokio::fs::write(&path, b"corrupt cache").await.unwrap();
    let result = cache
        .install(&artifact(bytes), package(&resolver), &cancel)
        .await;
    assert!(
        matches!(
            result,
            Err(RuntimeError::Artifact(
                demi_shared_artifacts::Error::Size { .. }
            ))
        ),
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
    let cache = ArtifactCache::new(cache_root.clone(), None, Installs::default())
        .await
        .unwrap();
    let result = cache
        .install(
            &artifact(b"small"),
            package(&resolver),
            &CancellationToken::new(),
        )
        .await;
    assert!(
        matches!(result, Err(RuntimeError::Artifact(_))),
        "{result:?}"
    );
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
    let cache = ArtifactCache::new(cache_root.clone(), None, Installs::default())
        .await
        .unwrap();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let result = cache
        .install(&artifact(b"bytes"), package(&resolver), &cancel)
        .await;
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

/// A resource's archive is unpacked into the cache, which a second install
/// takes unread, and its download is an install the runner reports, with
/// the resource's title, until it ends.
#[tokio::test]
async fn a_resource_is_unpacked_once_and_reported_while_it_downloads() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("chrome.zip");
    let bytes = demi_shared_artifacts::testing::zip(&[
        ("chrome-linux64/chrome", b"chrome"),
        ("chrome-linux64/LICENSE", b"license"),
    ]);
    tokio::fs::write(&source, &bytes).await.unwrap();
    let (release, released) = tokio::sync::oneshot::channel::<()>();
    let resolver = Gated {
        resolver: Resolver {
            path: source,
            calls: AtomicUsize::new(0),
        },
        gate: tokio::sync::Mutex::new(Some(released)),
    };
    let installs = Installs::default();
    let mut reported = installs.subscribe();
    let cache = ArtifactCache::new(root.path().join("cache"), None, installs)
        .await
        .unwrap();
    let digest = artifact(&bytes);
    let resource = ResourceArtifact {
        sha256: digest.sha256.clone(),
        size: digest.size,
        entry: "chrome-linux64/chrome".to_owned(),
    };
    let title = "Chrome for Testing 153.0.8010.36";
    let cancel = CancellationToken::new();
    let for_package = ForPackage {
        id: "demi.browser",
        resolver: &resolver,
    };
    let installing = cache.install_resource(title, &resource, for_package, &cancel);
    let watching = async {
        // The download waits for its location: it is an install already.
        while reported.current().is_empty() {
            assert!(reported.changed().await);
        }
        assert_eq!(
            reported.current(),
            [Install {
                package: "demi.browser".to_owned(),
                artifact: InstallArtifact::Resource {
                    title: title.to_owned(),
                },
                phase: InstallPhase::Download,
                done: 0,
                total: digest.size,
            }]
        );
        release.send(()).unwrap();
    };
    let (entry, ()) = tokio::join!(installing, watching);
    let entry = entry.unwrap();
    assert_eq!(
        entry,
        root.path()
            .join("cache")
            .join(&digest.sha256)
            .join("chrome-linux64/chrome")
    );
    assert_eq!(tokio::fs::read(&entry).await.unwrap(), b"chrome");
    assert_eq!(reported.current(), []);
    let again = cache
        .install_resource(title, &resource, for_package, &cancel)
        .await
        .unwrap();
    assert_eq!(again, entry);
    assert_eq!(resolver.resolver.calls.load(Ordering::SeqCst), 1);
}

/// A resolver that answers once its gate opens.
struct Gated {
    resolver: Resolver,
    gate: tokio::sync::Mutex<Option<tokio::sync::oneshot::Receiver<()>>>,
}

impl ArtifactResolver for Gated {
    fn resolve<'a>(
        &'a self,
        artifact: &'a PackageArtifact,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(async move {
            if let Some(gate) = self.gate.lock().await.take() {
                gate.await.unwrap();
            }
            self.resolver.resolve(artifact, cancel).await
        })
    }
}
