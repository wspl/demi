//! The verified artifact cache (`crates/runner-command-packages/src/cache.rs`): a miss downloads and verifies, a hit asks nobody, nothing
//! partial or mismatched is ever published, a download reports how it goes,
//! and a newer version of a line replaces the older ones no running service
//! holds.

use demi_command_protocol::{ArtifactForm, ArtifactProgress, PackageArtifact};
use demi_runner_command_packages::{
    ArtifactResolver, ArtifactSource, RuntimeError,
    cache::{ArtifactCache, SILENT, Wanted},
};
use futures_util::{FutureExt, future::BoxFuture};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};
use std::time::SystemTime;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

struct Resolver {
    path: PathBuf,
    calls: AtomicUsize,
}

impl Resolver {
    fn at(path: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            path,
            calls: AtomicUsize::new(0),
        })
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

async fn cache(root: &Path) -> Arc<ArtifactCache> {
    Arc::new(
        ArtifactCache::new(root.join("cache"), None, None)
            .await
            .unwrap(),
    )
}

/// Locates `path` only once the test opens it, and tells the test of each
/// call, so a test holds a download at its start.
struct Gated {
    path: PathBuf,
    calls: watch::Sender<usize>,
    open: watch::Sender<bool>,
}

impl Gated {
    fn at(path: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            path,
            calls: watch::Sender::new(0),
            open: watch::Sender::new(false),
        })
    }

    async fn called(&self) {
        let mut calls = self.calls.subscribe();
        calls.wait_for(|calls| *calls > 0).await.unwrap();
    }
}

impl ArtifactResolver for Gated {
    fn resolve<'a>(
        &'a self,
        _: &'a PackageArtifact,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(async move {
            self.calls.send_modify(|calls| *calls += 1);
            let mut open = self.open.subscribe();
            tokio::select! {
                _ = cancel.cancelled() => Err(RuntimeError::Cancelled),
                _ = open.wait_for(|open| *open) => Ok(ArtifactSource::Local(self.path.clone())),
            }
        })
    }
}

/// A stream's resolver whose first location is a URL that has expired by
/// the time it arrives, so the download asks for a location again; it
/// tells the test of each call, and once its stream has ended, as when its
/// caller left, it locates nothing more.
struct Expiring {
    calls: watch::Sender<usize>,
    ended: watch::Sender<bool>,
}

impl ArtifactResolver for Expiring {
    fn resolve<'a>(
        &'a self,
        _: &'a PackageArtifact,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(async move {
            let mut first = false;
            self.calls.send_modify(|calls| {
                first = *calls == 0;
                *calls += 1;
            });
            if first {
                return Ok(ArtifactSource::Url {
                    // Never fetched: the URL has expired.
                    url: "http://127.0.0.1:9/expired".to_owned(),
                    expires_at: Some(SystemTime::UNIX_EPOCH),
                });
            }
            let mut ended = self.ended.subscribe();
            tokio::select! {
                _ = cancel.cancelled() => Err(RuntimeError::Cancelled),
                _ = ended.wait_for(|ended| *ended) => {
                    Err(RuntimeError::Location("the stream has ended".into()))
                }
            }
        })
    }
}

/// The URL the stream of the conversation that started a download located
/// expired, and that conversation is stopped while the download asks for
/// the location again: the download asks the conversation still waiting,
/// rather than the closed stream, and that one gets the file
/// (`crates/runner-command-packages/src/cache.rs`). Cost: a few small files.
#[tokio::test]
async fn a_shared_download_asks_a_remaining_caller_again_for_its_location() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let bytes = b"claude code executable fixture";
    tokio::fs::write(&source, bytes).await.unwrap();
    let starting = Arc::new(Expiring {
        calls: watch::Sender::new(0),
        ended: watch::Sender::new(false),
    });
    let staying = Resolver::at(source);
    let cache = cache(root.path()).await;
    let wanted = file("Claude Code", "2.1.0", bytes);
    let stopped = CancellationToken::new();
    let waiting = CancellationToken::new();
    let mut first = Box::pin(cache.install(&wanted, starting.clone(), SILENT, &stopped));
    let mut second = Box::pin(cache.install(&wanted, staying.clone(), SILENT, &waiting));
    assert!(first.as_mut().now_or_never().is_none());
    assert!(second.as_mut().now_or_never().is_none());
    let mut calls = starting.calls.subscribe();
    tokio::select! {
        _ = calls.wait_for(|calls| *calls == 2) => {}
        result = &mut first => panic!("the first install ended before asking again: {result:?}"),
        result = &mut second => panic!("the second install ended before the download: {result:?}"),
    }
    stopped.cancel();
    let cancelled = first.await;
    assert!(matches!(cancelled, Err(RuntimeError::Cancelled)), "{cancelled:?}");
    starting.ended.send_replace(true);
    let path = second.await.unwrap();
    assert_eq!(tokio::fs::read(&path).await.unwrap(), bytes);
    assert_eq!(staying.calls.load(Ordering::SeqCst), 1, "the waiting one located it");
}

/// A download that failed is not handed to a conversation that asks for
/// the artifact afterwards, even while another caller of the failed one
/// has yet to read its failure: the later one downloads afresh and gets
/// the file. Cost: a few small files.
#[tokio::test]
async fn a_caller_after_a_failed_shared_download_starts_a_fresh_one() {
    let root = tempfile::tempdir().unwrap();
    let bytes = b"claude code executable fixture";
    let damaged = root.path().join("damaged");
    tokio::fs::write(&damaged, b"claude code executable FIXTURE").await.unwrap();
    let good = root.path().join("good");
    tokio::fs::write(&good, bytes).await.unwrap();
    let failing = Gated::at(damaged);
    let cache = cache(root.path()).await;
    let wanted = file("Claude Code", "2.1.0", bytes);
    let cancel = CancellationToken::new();
    let mut first = Box::pin(cache.install(&wanted, failing.clone(), SILENT, &cancel));
    let mut second = Box::pin(cache.install(&wanted, failing.clone(), SILENT, &cancel));
    assert!(first.as_mut().now_or_never().is_none());
    assert!(second.as_mut().now_or_never().is_none());
    failing.open.send_replace(true);
    let failed = first.await;
    // Shared, since the second holds the same failure.
    assert!(matches!(failed, Err(RuntimeError::Shared(_))), "{failed:?}");
    // The second still holds the failed download, unread.
    let fresh = Resolver::at(good);
    let path = cache
        .install(&wanted, fresh.clone(), SILENT, &cancel)
        .await
        .unwrap();
    assert_eq!(tokio::fs::read(&path).await.unwrap(), bytes);
    assert_eq!(fresh.calls.load(Ordering::SeqCst), 1);
    let shared = second.await;
    assert!(matches!(shared, Err(RuntimeError::Shared(_))), "{shared:?}");
}

/// The names in the cache directory, which holds no staged file once its
/// installs are done.
fn listed(root: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(root.join("cache"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

/// Five conversations that start Claude Code at once after a deploy share one
/// download of its executable instead of five copies that fill the disk, and
/// each gets the one published file.
///
/// An install joins the download as it is first polled, and the download is
/// held until all have joined, so polling each once makes them concurrent
/// however fast the machine.
///
/// Cost: a few small files.
#[tokio::test]
async fn concurrent_installs_of_one_artifact_share_one_download() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let bytes = b"claude code executable fixture";
    tokio::fs::write(&source, bytes).await.unwrap();
    let resolver = Gated::at(source);
    let cache = cache(root.path()).await;
    let cancel = CancellationToken::new();
    let wanted = file("Claude Code", "2.1.0", bytes);
    let mut installs: Vec<_> = (0..5)
        .map(|_| Box::pin(cache.install(&wanted, resolver.clone(), SILENT, &cancel)))
        .collect();
    for install in &mut installs {
        assert!(install.as_mut().now_or_never().is_none());
    }
    resolver.open.send_replace(true);
    let paths = futures_util::future::join_all(installs).await;
    let destination = root.path().join("cache").join(&wanted.artifact.sha256);
    for path in paths {
        assert_eq!(path.unwrap(), destination);
    }
    assert_eq!(*resolver.calls.borrow(), 1, "one download");
    assert_eq!(tokio::fs::read(&destination).await.unwrap(), bytes);
    let sha256 = &wanted.artifact.sha256;
    assert_eq!(listed(root.path()), [sha256.clone(), format!("{sha256}.line.json")]);
}

/// A conversation stopped while its stream locates the executable leaves
/// the download to another one that waits for it: the download asks that
/// one where the artifact is and goes on, and nothing staged is left
/// beside the file (`crates/runner-command-packages/src/cache.rs`). Cost: a few
/// small files.
#[tokio::test]
async fn cancelling_one_install_leaves_the_shared_download_to_the_others() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let bytes = b"claude code executable fixture";
    tokio::fs::write(&source, bytes).await.unwrap();
    let starting = Gated::at(source.clone());
    let staying = Resolver::at(source);
    let cache = cache(root.path()).await;
    let wanted = file("Claude Code", "2.1.0", bytes);
    let stopped = CancellationToken::new();
    let waiting = CancellationToken::new();
    let mut first = Box::pin(cache.install(&wanted, starting.clone(), SILENT, &stopped));
    let mut second = Box::pin(cache.install(&wanted, staying.clone(), SILENT, &waiting));
    assert!(first.as_mut().now_or_never().is_none());
    assert!(second.as_mut().now_or_never().is_none());
    tokio::select! {
        _ = starting.called() => {}
        result = &mut first => panic!("the first install ended at its start: {result:?}"),
        result = &mut second => panic!("the second install downloaded on its own: {result:?}"),
    }
    stopped.cancel();
    let cancelled = first.await;
    assert!(matches!(cancelled, Err(RuntimeError::Cancelled)), "{cancelled:?}");
    // A location the first one's stream gave now would not be asked for.
    starting.open.send_replace(true);
    let path = second.await.unwrap();
    assert_eq!(tokio::fs::read(&path).await.unwrap(), bytes);
    assert_eq!(*starting.calls.borrow(), 1);
    assert_eq!(
        staying.calls.load(Ordering::SeqCst),
        1,
        "the waiting one locates it once the first left, in the same download"
    );
    let sha256 = &wanted.artifact.sha256;
    assert_eq!(listed(root.path()), [sha256.clone(), format!("{sha256}.line.json")]);
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
    let path = cache.install(&wanted, resolver.clone(), SILENT, &cancel).await.unwrap();
    assert_eq!(tokio::fs::read(&path).await.unwrap(), bytes);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o111, 0o111, "the cached file runs");
    }
    assert_eq!(
        cache.install(&wanted, resolver.clone(), SILENT, &cancel).await.unwrap(),
        path
    );
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
    tokio::fs::write(&path, b"corrupt cache").await.unwrap();
    let result = cache.install(&wanted, resolver.clone(), SILENT, &cancel).await;
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
            resolver.clone(),
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
            resolver.clone(),
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
/// unpacking (`crates/runner-command-packages/src/cache.rs`); it is unpacked into
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
        .install(&wanted, resolver.clone(), &report, &cancel)
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
            .install(&wanted, resolver.clone(), &report, &cancel)
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
        paths.push(cache.install(&wanted, resolver.clone(), SILENT, &cancel).await.unwrap());
    }
    let other_source = root.path().join("other");
    tokio::fs::write(&other_source, b"program").await.unwrap();
    let other = file("program", "0.1.3", b"program");
    let other_path = cache
        .install(&other, Resolver::at(other_source), SILENT, &cancel)
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
        .install(&wanted, Resolver::at(three), SILENT, &cancel)
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
        .install(&wanted, Resolver::at(root.path().join("3")), SILENT, &cancel)
        .await
        .unwrap();
    assert!(!paths[1].exists(), "version 2 goes once nothing holds it");
}

/// Locates the artifact at one URL.
struct At(String);

impl ArtifactResolver for At {
    fn resolve<'a>(
        &'a self,
        _: &'a PackageArtifact,
        _: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(async move {
            Ok(ArtifactSource::Url {
                url: self.0.clone(),
                expires_at: None,
            })
        })
    }
}

/// A download whose connection breaks off halfway resumes on a new one with
/// a Range request from the bytes it holds, in the content coding the store
/// serves, and installs the executable those bytes decode to; its staging
/// file goes once it is checked (`crates/runner-command-packages/src/cache.rs`).
/// Cost: two local connections and a small file.
#[tokio::test]
async fn a_download_that_breaks_off_resumes_from_its_bytes() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let executable: Vec<u8> = (0u32..4096)
        .flat_map(|block| Sha256::digest(block.to_le_bytes()))
        .collect();
    let encoded = demi_shared_artifacts::encode_blocking(
        &executable,
        demi_shared_artifacts::Effort::Fast,
    )
    .unwrap();
    let half = encoded.len() / 2;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/native-artifacts/x", listener.local_addr().unwrap());
    let serving = tokio::spawn({
        let encoded = encoded.clone();
        async move {
            let mut requests = Vec::new();
            for answer in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 4096];
                let read = socket.read(&mut request).await.unwrap();
                requests.push(String::from_utf8_lossy(&request[..read]).to_lowercase());
                let size = encoded.len();
                if answer == 0 {
                    // The connection breaks off halfway.
                    let head = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {size}\r\ncontent-encoding: zstd\r\n\r\n"
                    );
                    socket.write_all(head.as_bytes()).await.unwrap();
                    socket.write_all(&encoded[..half]).await.unwrap();
                } else {
                    let head = format!(
                        "HTTP/1.1 206 Partial Content\r\ncontent-length: {}\r\ncontent-range: bytes {half}-{}/{size}\r\ncontent-encoding: zstd\r\nconnection: close\r\n\r\n",
                        size - half,
                        size - 1
                    );
                    socket.write_all(head.as_bytes()).await.unwrap();
                    socket.write_all(&encoded[half..]).await.unwrap();
                }
            }
            requests
        }
    });
    let root = tempfile::tempdir().unwrap();
    let cache = cache(root.path()).await;
    let wanted = file("program", "1.0.0", &executable);
    let path = cache
        .install(
            &wanted,
            Arc::new(At(url)),
            SILENT,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert_eq!(tokio::fs::read(&path).await.unwrap(), executable);
    let requests = serving.await.unwrap();
    assert!(requests[0].contains("accept-encoding: zstd"), "{}", requests[0]);
    assert!(
        requests[1].contains(&format!("range: bytes={half}-")),
        "{}",
        requests[1]
    );
    let staging = root
        .path()
        .join("cache")
        .join(format!(".download-{}", wanted.artifact.sha256));
    assert!(!staging.exists());
}

/// A download that sends more bytes than any form of the artifact can have
/// fails as they arrive, without a declared length to refuse first, rather
/// than when the body ends: this one never does (`crates/shared-artifacts/src/download.rs`). Cost: one local connection.
#[tokio::test]
async fn more_bytes_than_the_artifact_can_have_fail_as_they_arrive() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let executable = b"a small native executable".to_vec();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/native-artifacts/x", listener.local_addr().unwrap());
    let serving = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 4096];
        let _read = socket.read(&mut request).await.unwrap();
        // No length, and bytes until the client hangs up.
        let _written = socket.write_all(b"HTTP/1.1 200 OK\r\nconnection: close\r\n\r\n").await;
        while socket.write_all(&[0; 64 * 1024]).await.is_ok() {}
    });
    let root = tempfile::tempdir().unwrap();
    let cache = cache(root.path()).await;
    let wanted = file("program", "1.0.0", &executable);
    let cancel = CancellationToken::new();
    let installing = cache.install(&wanted, Arc::new(At(url)), SILENT, &cancel);
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), installing)
        .await
        .expect("the download stops as the bytes pass the bound");
    assert!(
        matches!(
            result,
            Err(RuntimeError::Artifact(demi_shared_artifacts::Error::TooLarge { .. }))
        ),
        "{result:?}"
    );
    serving.abort();
}

/// A download cancelled midway keeps its staging file for a resume, and an
/// install of another version of its line removes it (`crates/shared-artifacts/src/download.rs`). Cost: one local connection and two small files.
#[tokio::test]
async fn a_newer_version_prunes_the_staging_file_of_an_older_one() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let older: Vec<u8> = (0u32..4096)
        .flat_map(|block| Sha256::digest(block.to_le_bytes()))
        .collect();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/native-artifacts/x", listener.local_addr().unwrap());
    let served = older.clone();
    let serving = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 4096];
        let _read = socket.read(&mut request).await.unwrap();
        let head = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", served.len());
        socket.write_all(head.as_bytes()).await.unwrap();
        socket.write_all(&served[..served.len() / 2]).await.unwrap();
        // The rest never comes; the connection stays open.
        std::future::pending::<()>().await;
    });
    let root = tempfile::tempdir().unwrap();
    let cache = cache(root.path()).await;
    let wanted = file("program", "1.0.0", &older);
    let staging = root
        .path()
        .join("cache")
        .join(format!(".download-{}", wanted.artifact.sha256));
    let cancel = CancellationToken::new();
    let arrived = watch::Sender::new(false);
    let report = |_| {
        arrived.send_replace(true);
    };
    let installing = cache.install(&wanted, Arc::new(At(url)), &report, &cancel);
    let mut heard = arrived.subscribe();
    let cancelled = tokio::join!(installing, async {
        heard.wait_for(|arrived| *arrived).await.unwrap();
        cancel.cancel();
    })
    .0;
    assert!(matches!(cancelled, Err(RuntimeError::Cancelled)), "{cancelled:?}");
    assert!(staging.exists(), "a cancelled download keeps its bytes");
    let newer = b"the newer executable";
    let source = root.path().join("newer");
    tokio::fs::write(&source, newer).await.unwrap();
    cache
        .install(
            &file("program", "1.1.0", newer),
            Resolver::at(source),
            SILENT,
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(!staging.exists(), "the older version's staging file stays");
    serving.abort();
}

/// Hands out a signed URL that expires `after` its first location, then
/// a renewed one that does not.
struct Signed {
    first: String,
    renewed: String,
    calls: AtomicUsize,
    after: std::time::Duration,
}

impl ArtifactResolver for Signed {
    fn resolve<'a>(
        &'a self,
        _: &'a PackageArtifact,
        _: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(async move {
            let first = self.calls.fetch_add(1, Ordering::SeqCst) == 0;
            Ok(ArtifactSource::Url {
                url: if first { self.first.clone() } else { self.renewed.clone() },
                expires_at: Some(SystemTime::now() + if first { self.after } else { std::time::Duration::from_secs(300) }),
            })
        })
    }
}

/// A signed URL that expires during a long download is refused when the
/// download resumes; the address is asked for again and the download
/// continues from the renewed one with a range, keeping the bytes it had
/// (`crates/shared-artifacts/src/download.rs`). Cost: three local connections and
/// the first URL's lifetime, a fifth of a second.
#[tokio::test]
async fn a_signed_url_that_expires_midway_continues_from_the_renewed_one() {
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    let executable: Vec<u8> = (0u32..4096)
        .flat_map(|block| Sha256::digest(block.to_le_bytes()))
        .collect();
    let half = executable.len() / 2;
    let size = executable.len();
    let lifetime = std::time::Duration::from_millis(200);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let served = executable.clone();
    let serving = tokio::spawn(async move {
        let mut requests = Vec::new();
        for answer in 0..3 {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let read = socket.read(&mut request).await.unwrap();
            requests.push(String::from_utf8_lossy(&request[..read]).to_lowercase());
            match answer {
                0 => {
                    // Half the bytes, then the connection breaks once the
                    // URL has expired.
                    let head = format!("HTTP/1.1 200 OK\r\ncontent-length: {size}\r\n\r\n");
                    socket.write_all(head.as_bytes()).await.unwrap();
                    socket.write_all(&served[..half]).await.unwrap();
                    tokio::time::sleep(lifetime * 2).await;
                }
                1 => {
                    let head = "HTTP/1.1 403 Forbidden\r\ncontent-length: 0\r\n\r\n";
                    socket.write_all(head.as_bytes()).await.unwrap();
                }
                _ => {
                    let head = format!(
                        "HTTP/1.1 206 Partial Content\r\ncontent-range: bytes {half}-{}/{size}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        size - 1,
                        size - half
                    );
                    socket.write_all(head.as_bytes()).await.unwrap();
                    socket.write_all(&served[half..]).await.unwrap();
                }
            }
        }
        requests
    });
    let resolver = Arc::new(Signed {
        first: format!("{base}/expiring"),
        renewed: format!("{base}/renewed"),
        calls: AtomicUsize::new(0),
        after: lifetime,
    });
    let root = tempfile::tempdir().unwrap();
    let cache = cache(root.path()).await;
    let wanted = file("program", "1.0.0", &executable);
    let path = cache
        .install(&wanted, resolver.clone(), SILENT, &CancellationToken::new())
        .await
        .unwrap();
    assert!(tokio::fs::read(&path).await.unwrap() == executable, "the installed bytes differ");
    let requests = serving.await.unwrap();
    assert!(requests[2].starts_with("get /renewed"), "{}", requests[2]);
    assert!(requests[2].contains(&format!("range: bytes={half}-")), "{}", requests[2]);
    assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
}
