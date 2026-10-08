//! The verified artifact cache (`native-runtime.md` § Install artifacts,
//! § The cache): a miss downloads and verifies, a hit asks nobody, nothing
//! partial or mismatched is ever published, a download reports how it goes,
//! and a newer version of a line replaces the older ones no running service
//! holds.

use demi_command_protocol::{ArtifactForm, ArtifactProgress, PackageArtifact};
use demi_runner_command_packages::{
    ArtifactResolver, ArtifactSource, RuntimeError,
    cache::{ArtifactCache, SILENT, Wanted},
};
use futures_util::future::BoxFuture;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio_util::sync::CancellationToken;

struct Resolver {
    path: PathBuf,
    calls: AtomicUsize,
}

impl Resolver {
    fn at(path: PathBuf) -> Self {
        Self {
            path,
            calls: AtomicUsize::new(0),
        }
    }
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

/// `bytes` as the `file` named `name` at `version` of `demi.fixture`.
fn file<'a>(name: &'a str, version: &'a str, bytes: &[u8]) -> Wanted<'a> {
    Wanted {
        package: "demi.fixture",
        name,
        version,
        artifact: artifact(bytes),
        form: &ArtifactForm::File,
    }
}

async fn cache(root: &Path) -> ArtifactCache {
    ArtifactCache::new(root.join("cache"), None, None)
        .await
        .unwrap()
}

/// An entry was verified as it was published, so a second install reuses it
/// without asking the backend; an entry of another size is not the one
/// declared.
#[tokio::test]
async fn a_cached_file_is_reused_without_asking_and_one_of_another_size_fails() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let bytes = b"native executable fixture";
    tokio::fs::write(&source, bytes).await.unwrap();
    let resolver = Resolver::at(source);
    let cache = cache(root.path()).await;
    let cancel = CancellationToken::new();
    let wanted = file("program", "1.0.0", bytes);
    let path = cache.install(&wanted, &resolver, SILENT, &cancel).await.unwrap();
    assert_eq!(tokio::fs::read(&path).await.unwrap(), bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "the cached file runs");
    }
    assert_eq!(
        cache.install(&wanted, &resolver, SILENT, &cancel).await.unwrap(),
        path
    );
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    tokio::fs::write(&path, b"corrupt cache").await.unwrap();
    let result = cache.install(&wanted, &resolver, SILENT, &cancel).await;
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

/// Nothing is left of a download whose bytes are not the declared ones, or
/// of an install cancelled before it began.
#[tokio::test]
async fn a_mismatched_or_cancelled_install_leaves_nothing_behind() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    tokio::fs::write(&source, b"too much data").await.unwrap();
    let resolver = Resolver::at(source);
    let cache = cache(root.path()).await;
    let mismatched = cache
        .install(
            &file("program", "1.0.0", b"small"),
            &resolver,
            SILENT,
            &CancellationToken::new(),
        )
        .await;
    assert!(
        matches!(mismatched, Err(RuntimeError::Artifact(_))),
        "{mismatched:?}"
    );
    let cancel = CancellationToken::new();
    cancel.cancel();
    let cancelled = cache
        .install(
            &file("program", "1.0.0", b"too much data"),
            &resolver,
            SILENT,
            &cancel,
        )
        .await;
    assert!(
        matches!(cancelled, Err(RuntimeError::Cancelled)),
        "{cancelled:?}"
    );
    assert!(
        std::fs::read_dir(root.path().join("cache"))
            .unwrap()
            .next()
            .is_none()
    );
}

/// An archive's download reports each tenth of its size once, then its
/// unpacking (`native-runtime.md` § Install artifacts); it is unpacked into
/// the cache, which a second install takes unread and reports nothing of.
#[tokio::test]
async fn an_archive_reports_its_download_by_tenths_and_is_unpacked_once() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("chrome.zip");
    // About a megabyte that does not compress, so the copy writes it in
    // many pieces, each well under a tenth.
    let license: Vec<u8> = (0u32..32_768)
        .flat_map(|block| Sha256::digest(block.to_le_bytes()))
        .collect();
    let bytes = demi_shared_artifacts::testing::zip(&[
        ("chrome-linux64/chrome", b"chrome"),
        ("chrome-linux64/LICENSE", &license),
    ]);
    tokio::fs::write(&source, &bytes).await.unwrap();
    let resolver = Resolver::at(source);
    let cache = cache(root.path()).await;
    let form = ArtifactForm::Archive {
        entry: "chrome-linux64/chrome".to_owned(),
    };
    let wanted = Wanted {
        package: "demi.browser",
        name: "Chrome for Testing",
        version: "153.0.8010.36",
        artifact: artifact(&bytes),
        form: &form,
    };
    let cancel = CancellationToken::new();
    let reported = std::sync::Mutex::new(Vec::new());
    let report = |progress| reported.lock().unwrap().push(progress);
    let entry = cache
        .install(&wanted, &resolver, &report, &cancel)
        .await
        .unwrap();
    let size = wanted.artifact.size;
    let reported = std::mem::take(&mut *reported.lock().unwrap());
    let (unpack, downloads) = reported.split_last().unwrap();
    assert_eq!(*unpack, ArtifactProgress::Unpack);
    let tenths: Vec<u64> = downloads
        .iter()
        .map(|progress| match progress {
            ArtifactProgress::Download { done, total } => {
                assert_eq!(*total, size);
                done * 10 / size
            }
            ArtifactProgress::Unpack => panic!("unpacking before the download ends"),
        })
        .collect();
    assert_eq!(tenths, (1..=10).collect::<Vec<_>>());
    assert_eq!(
        downloads.last(),
        Some(&ArtifactProgress::Download {
            done: size,
            total: size
        })
    );
    assert_eq!(
        entry,
        root.path()
            .join("cache")
            .join(&wanted.artifact.sha256)
            .join("chrome-linux64/chrome")
    );
    assert_eq!(tokio::fs::read(&entry).await.unwrap(), b"chrome");
    let again = std::sync::Mutex::new(Vec::new());
    let report = |progress| again.lock().unwrap().push(progress);
    assert_eq!(
        cache
            .install(&wanted, &resolver, &report, &cancel)
            .await
            .unwrap(),
        entry
    );
    assert_eq!(*again.lock().unwrap(), []);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
}

/// Installing a newer version of a line removes the older one, unless a
/// running service holds it, which goes at a later install; the line's
/// artifacts are listed newest first, and another line is left alone.
#[tokio::test]
async fn a_newer_version_replaces_the_older_ones_no_service_holds() {
    let root = tempfile::tempdir().unwrap();
    let cache = cache(root.path()).await;
    let cancel = CancellationToken::new();
    let mut paths = Vec::new();
    for (version, bytes) in [("1", b"cli one".as_slice()), ("2", b"cli two")] {
        let source = root.path().join(version);
        tokio::fs::write(&source, bytes).await.unwrap();
        let resolver = Resolver::at(source);
        let wanted = file("Claude Code", version, bytes);
        paths.push(cache.install(&wanted, &resolver, SILENT, &cancel).await.unwrap());
    }
    let other_source = root.path().join("other");
    tokio::fs::write(&other_source, b"program").await.unwrap();
    let other = file("program", "0.1.3", b"program");
    let other_path = cache
        .install(&other, &Resolver::at(other_source), SILENT, &cancel)
        .await
        .unwrap();
    assert!(!paths[0].exists(), "version 1 is replaced");
    let listed: Vec<String> = cache
        .installed("demi.fixture", "Claude Code")
        .await
        .unwrap()
        .into_iter()
        .map(|installed| installed.version)
        .collect();
    assert_eq!(listed, ["2"]);
    assert!(other_path.exists(), "another line is left alone");
    // A held version stays when a newer one installs, and goes at the next.
    let held = cache.holds().hold(&artifact(b"cli two").sha256);
    let three = root.path().join("3");
    tokio::fs::write(&three, b"cli three").await.unwrap();
    let wanted = file("Claude Code", "3", b"cli three");
    cache
        .install(&wanted, &Resolver::at(three), SILENT, &cancel)
        .await
        .unwrap();
    let listed: Vec<String> = cache
        .installed("demi.fixture", "Claude Code")
        .await
        .unwrap()
        .into_iter()
        .map(|installed| installed.version)
        .collect();
    assert_eq!(listed, ["3", "2"]);
    assert!(paths[1].exists());
    drop(held);
    cache
        .install(&wanted, &Resolver::at(root.path().join("3")), SILENT, &cancel)
        .await
        .unwrap();
    assert!(!paths[1].exists(), "version 2 goes once nothing holds it");
}
