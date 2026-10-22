//! The verified artifact cache (`crates/runner-command-packages/src/cache.rs`): a miss downloads and verifies, a hit asks nobody, nothing
//! partial or mismatched is ever published, a download reports how it goes,
//! and a newer version of a line replaces the older ones no running service
//! holds.

use demi_command_protocol::{ArtifactForm, PackageArtifact};
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
