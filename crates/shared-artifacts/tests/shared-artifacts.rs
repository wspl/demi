//! Verified and measured downloads, copies, digests, publication, release
//! publication, the install lock, receipts and archive installation.

use std::{
    path::{Path, PathBuf},
    time::Duration,
};

use demi_shared_artifacts::{
    Archive, ArchiveInstall, CONTENT_CODING, Client, Digest, Download, Effort, Error, InstallLock,
    Mode, Patience, Permissions, Publication, ReleaseFile, ReleaseRecord, Staged, Verifier,
    check_encoded_blocking, copy, digest, download, download_client, download_client_allowing_http,
    encode_blocking,
    install_archive, installed,
    publish, publish_bytes, publish_directory, publish_release, receipt, recorded,
    holds,
    testing::{Answer, Server, tar_zst, zip},
};
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

const BODY: &[u8] = b"verified bytes";

fn declared(bytes: &[u8]) -> Digest {
    Digest {
        size: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
    }
}

/// A fixture server that answers `/artifact` with `status` and `body`,
/// declaring its length when `length` says so.
async fn serve(status: u16, body: &[u8], length: bool) -> Server {
    let answer = Answer {
        status,
        length,
        ..Answer::ok(body)
    };
    Server::start([("/artifact".to_owned(), answer)]).await
}

/// Downloads `request` through a staging file in a temporary directory and
/// answers its bytes, decoded and checked against `expected`.
async fn fetch(
    client: &Client,
    request: Download<'_>,
    expected: &Digest,
    cancel: &CancellationToken,
) -> Result<Vec<u8>, Error> {
    let directory = tempfile::tempdir().unwrap();
    let staging = directory.path().join("download");
    let received = download(client, request, &staging, cancel, |_, _| {}).await?;
    let mut output = Vec::new();
    received.verify(&staging, expected, &mut output, cancel).await?;
    assert!(!staging.exists(), "the staging file goes once checked");
    Ok(output)
}

/// `url`, as a fixture server serves it.
fn plain(url: &str) -> Download<'_> {
    Download::new(url)
}

#[tokio::test]
async fn a_download_is_checked_once_complete() {
    let client = download_client_allowing_http().unwrap();
    let cancel = CancellationToken::new();
    let server = serve(200, BODY, true).await;
    let url = server.url("/artifact");
    let output = fetch(&client, plain(&url), &declared(BODY), &cancel).await.unwrap();
    assert_eq!(output, BODY);
    let wrong = Digest {
        sha256: "0".repeat(64),
        ..declared(BODY)
    };
    let result = fetch(&client, plain(&url), &wrong, &cancel).await;
    assert!(matches!(result, Err(Error::Digest)), "{result:?}");
    let short = Digest {
        size: 3,
        ..declared(BODY)
    };
    let result = fetch(&client, plain(&url), &short, &cancel).await;
    assert!(
        matches!(result, Err(Error::TooLarge { declared: 3 } | Error::Size { declared: 3, .. })),
        "{result:?}"
    );
    let missing = server.url("/elsewhere");
    let result = fetch(&client, plain(&missing), &declared(BODY), &cancel).await;
    assert!(
        matches!(result, Err(Error::Rejected { status: 404 })),
        "{result:?}"
    );
    // A redirect that names no location is no answer.
    let moved = serve(302, b"", true).await;
    let moved = moved.url("/artifact");
    let result = fetch(&client, plain(&moved), &declared(BODY), &cancel).await;
    assert!(
        matches!(result, Err(Error::Rejected { status: 302 })),
        "{result:?}"
    );
    cancel.cancel();
    let result = fetch(&client, plain(&url), &declared(BODY), &cancel).await;
    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
}

/// A command executable arrives in zstd content coding, and what is checked
/// against its declared size and SHA-256 is the decoded executable; a coding
/// the download does not decode fails instead of producing other bytes.
#[tokio::test]
async fn a_download_decodes_zstd_before_it_verifies_and_refuses_another_coding() {
    let client = download_client_allowing_http().unwrap();
    let cancel = CancellationToken::new();
    let encoded = encode_blocking(BODY, Effort::Published).unwrap();
    assert_ne!(encoded, BODY);
    let server = Server::start([
        (
            "/zstd".to_owned(),
            Answer {
                coding: Some(CONTENT_CODING),
                ..Answer::ok(encoded)
            },
        ),
        (
            "/gzip".to_owned(),
            Answer {
                coding: Some("gzip"),
                ..Answer::ok(BODY)
            },
        ),
    ])
    .await;
    let zstd = server.url("/zstd");
    let output = fetch(&client, plain(&zstd), &declared(BODY), &cancel).await.unwrap();
    assert_eq!(output, BODY);
    let gzip = server.url("/gzip");
    let result = fetch(&client, plain(&gzip), &declared(BODY), &cancel).await;
    assert!(
        matches!(&result, Err(Error::Coding(coding)) if coding == "gzip"),
        "{result:?}"
    );
}

/// A download holds no more than its caller's limit: a declared length past
/// it fails before the body, and without one, bytes past it stop the
/// download.
#[tokio::test]
async fn a_download_holds_no_more_than_its_limit() {
    let client = download_client_allowing_http().unwrap();
    let cancel = CancellationToken::new();
    for length in [true, false] {
        let server = serve(200, BODY, length).await;
        let url = server.url("/artifact");
        let request = Download {
            limit: 3,
            ..plain(&url)
        };
        let result = fetch(&client, request, &declared(BODY), &cancel).await;
        assert!(
            matches!(result, Err(Error::TooLarge { declared: 3 })),
            "{result:?}"
        );
    }
}

#[tokio::test]
async fn the_download_client_refuses_plain_http() {
    let client = download_client().unwrap();
    let server = serve(200, BODY, true).await;
    let url = server.url("/artifact");
    let result = fetch(&client, Download::new(&url), &declared(BODY), &CancellationToken::new()).await;
    assert!(matches!(result, Err(Error::Download(_))), "{result:?}");
}

#[tokio::test]
async fn copies_and_digests_check_the_declared_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    std::fs::write(&source, BODY).unwrap();
    let cancel = CancellationToken::new();
    assert_eq!(
        digest(&source, 1024, &cancel).await.unwrap(),
        declared(BODY)
    );
    assert!(matches!(
        digest(&source, 3, &cancel).await,
        Err(Error::TooLarge { declared: 3 })
    ));
    let mut output = Vec::new();
    let mut input = tokio::fs::File::open(&source).await.unwrap();
    copy(&mut input, &declared(BODY), &mut output, &cancel)
        .await
        .unwrap();
    assert_eq!(output, BODY);
    let mut input = tokio::fs::File::open(&source).await.unwrap();
    let longer = Digest {
        size: BODY.len() as u64 + 1,
        ..declared(BODY)
    };
    let result = copy(&mut input, &longer, &mut Vec::new(), &cancel).await;
    assert!(matches!(result, Err(Error::Size { .. })), "{result:?}");
    let mut verifier = Verifier::new(&declared(b"ab"));
    verifier.update(b"a").unwrap();
    verifier.update(b"b").unwrap();
    assert!(verifier.update(b"c").is_err());
    cancel.cancel();
    assert!(matches!(
        digest(&source, 1024, &cancel).await,
        Err(Error::Cancelled)
    ));
}

fn publication(mode: Mode, permissions: Permissions) -> Publication {
    Publication {
        mode,
        permissions,
        durable: true,
    }
}

#[tokio::test]
async fn publication_creates_or_replaces_whole_files() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("file");
    publish_bytes(
        &path,
        b"first",
        publication(Mode::CreateNew, Permissions::Default),
    )
    .await
    .unwrap();
    let again = publish_bytes(
        &path,
        b"second",
        publication(Mode::CreateNew, Permissions::Default),
    )
    .await;
    assert!(
        matches!(&again, Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists),
        "{again:?}"
    );
    assert_eq!(std::fs::read(&path).unwrap(), b"first");
    let mut input: &[u8] = b"replaced";
    publish(
        &path,
        &mut input,
        publication(Mode::Replace, Permissions::Default),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"replaced");
    // Nothing but the file remains beside it.
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    // A cancelled publication leaves the old file and no temporary.
    let cancel = CancellationToken::new();
    cancel.cancel();
    let mut input: &[u8] = b"never";
    let result = publish(
        &path,
        &mut input,
        publication(Mode::Replace, Permissions::Default),
        &cancel,
    )
    .await;
    assert!(matches!(result, Err(Error::Cancelled)));
    assert_eq!(std::fs::read(&path).unwrap(), b"replaced");
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[tokio::test]
async fn a_cancelled_publication_publishes_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("output");
    let cancel = CancellationToken::new();
    cancel.cancel();
    // The whole input is ready at once, so only the cancellation stops it.
    let result = publish(
        &path,
        &mut std::io::Cursor::new(BODY.to_vec()),
        publication(Mode::CreateNew, Permissions::Default),
        &cancel,
    )
    .await;
    assert!(matches!(result, Err(Error::Cancelled)));
    // Neither the file nor its staged copy is left.
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
}

#[tokio::test]
async fn a_staged_file_appears_only_when_published() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("tool");
    let mut abandoned = Staged::new(&path, publication(Mode::CreateNew, Permissions::Executable))
        .await
        .unwrap();
    abandoned.file().write_all(b"partial").await.unwrap();
    // Its name says whose it is, should its process die before it is
    // published (`crates/runner-host/src/files.rs`).
    let names: Vec<String> = std::fs::read_dir(directory.path())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert!(
        matches!(&names[..], [name] if name.starts_with(".demi-partial-")),
        "{names:?}"
    );
    drop(abandoned);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    let mut staged = Staged::new(&path, publication(Mode::CreateNew, Permissions::Executable))
        .await
        .unwrap();
    let mut input = tokio::fs::File::open(env!("CARGO_MANIFEST_DIR").to_owned() + "/Cargo.toml")
        .await
        .unwrap();
    let expected = std::fs::read(env!("CARGO_MANIFEST_DIR").to_owned() + "/Cargo.toml").unwrap();
    copy(
        &mut input,
        &declared(&expected),
        staged.file(),
        &CancellationToken::new(),
    )
    .await
    .unwrap();
    assert!(!path.exists());
    staged.publish().await.unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), expected);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[tokio::test]
async fn publication_sets_or_keeps_permissions() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    let private = directory.path().join("private");
    publish_bytes(
        &private,
        b"secret",
        publication(Mode::CreateNew, Permissions::Private),
    )
    .await
    .unwrap();
    assert_eq!(mode(&private), 0o600);
    let executable = directory.path().join("tool");
    publish_bytes(
        &executable,
        b"#!/bin/sh\n",
        publication(Mode::CreateNew, Permissions::Executable),
    )
    .await
    .unwrap();
    assert_eq!(mode(&executable), 0o755);
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o640)).unwrap();
    publish_bytes(
        &private,
        b"rotated",
        publication(Mode::Replace, Permissions::Keep),
    )
    .await
    .unwrap();
    assert_eq!(mode(&private), 0o640);
}

#[tokio::test]
async fn a_staged_directory_replaces_the_installed_one() {
    let root = tempfile::tempdir().unwrap();
    let installed = root.path().join("1.0.0");
    std::fs::create_dir(&installed).unwrap();
    std::fs::write(installed.join("old"), b"old").unwrap();
    let staged = root.path().join("staged");
    std::fs::create_dir(&staged).unwrap();
    std::fs::write(staged.join("new"), b"new").unwrap();
    publish_directory(&staged, &installed).await.unwrap();
    assert!(!staged.exists());
    assert!(!installed.join("old").exists());
    assert_eq!(std::fs::read(installed.join("new")).unwrap(), b"new");
}

#[tokio::test]
async fn a_release_is_published_whole_once_and_refused_over_other_contents() {
    let root = tempfile::tempdir().unwrap();
    let sources = root.path().join("sources");
    std::fs::create_dir(&sources).unwrap();
    std::fs::write(sources.join("tool"), b"tool v1").unwrap();
    std::fs::write(sources.join("data"), b"data").unwrap();
    let file = |name: &str, path: &str, bytes: &[u8], executable: bool| ReleaseFile {
        source: sources.join(name),
        path: path.into(),
        digest: declared(bytes),
        executable,
    };
    let files = [
        file("tool", "x86_64-unknown-linux-musl/tool", b"tool v1", true),
        file("data", "data", b"data", false),
    ];
    let record = ReleaseRecord {
        name: "descriptor.json",
        bytes: b"{\"version\":\"1\"}\n",
    };
    let releases = root.path().join("releases");
    let directory = releases.join("tool-1");
    let cancel = CancellationToken::new();
    publish_release(&directory, record, &files, &cancel)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read(directory.join("descriptor.json")).unwrap(),
        record.bytes
    );
    assert_eq!(
        std::fs::read(directory.join("x86_64-unknown-linux-musl/tool")).unwrap(),
        b"tool v1"
    );
    assert_eq!(std::fs::read(directory.join("data")).unwrap(), b"data");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o111;
        assert_eq!(
            mode(&directory.join("x86_64-unknown-linux-musl/tool")),
            0o111
        );
        assert_eq!(mode(&directory.join("data")), 0);
    }
    // The same release again is the one in place.
    publish_release(&directory, record, &files, &cancel)
        .await
        .unwrap();
    // Another record under the same name is refused, and so is a file that
    // changed in place; nothing in place changes.
    let other = ReleaseRecord {
        name: "descriptor.json",
        bytes: b"{\"version\":\"2\"}\n",
    };
    let refused = publish_release(&directory, other, &files, &cancel).await;
    assert!(
        matches!(&refused, Err(Error::Conflict(path)) if path.ends_with("descriptor.json")),
        "{refused:?}"
    );
    std::fs::write(directory.join("data"), b"corrupt").unwrap();
    let refused = publish_release(&directory, record, &files, &cancel).await;
    assert!(
        matches!(&refused, Err(Error::Conflict(path)) if path.ends_with("data")),
        "{refused:?}"
    );
    assert_eq!(
        std::fs::read(directory.join("descriptor.json")).unwrap(),
        record.bytes
    );
    // A source that is not what its release declares publishes nothing.
    std::fs::write(sources.join("tool"), b"tool v2").unwrap();
    let changed = publish_release(&releases.join("tool-2"), record, &files, &cancel).await;
    assert!(matches!(changed, Err(Error::Digest)), "{changed:?}");
    // No stage is left beside the releases, whatever happened.
    let names: Vec<String> = std::fs::read_dir(&releases)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert_eq!(names, ["tool-1"]);
}

/// The clock is paused: a waiter tries the lock again every 50 ms of it, so
/// each sleep below lets it find the lock held a few times, at once.
#[tokio::test(start_paused = true)]
async fn one_installer_holds_the_lock_and_a_waiter_can_give_up() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("artifact.lock");
    let cancel = CancellationToken::new();
    let held = InstallLock::acquire(&path, &cancel).await.unwrap();
    let waiting = tokio::spawn({
        let path = path.clone();
        async move { InstallLock::acquire(&path, &CancellationToken::new()).await }
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(!waiting.is_finished());
    drop(held);
    let next = tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .expect("the lock passes on")
        .unwrap()
        .unwrap();
    let giving_up = CancellationToken::new();
    let quitter = tokio::spawn({
        let path = path.clone();
        let giving_up = giving_up.clone();
        async move { InstallLock::acquire(&path, &giving_up).await }
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    giving_up.cancel();
    let quit = tokio::time::timeout(Duration::from_secs(5), quitter)
        .await
        .expect("a waiter gives up when asked");
    assert!(matches!(quit.unwrap(), Err(Error::Cancelled)));
    drop(next);
}

/// The names in `directory`, sorted.
fn names(directory: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Installs `archive` from `url` into `root`, as a runner installs a
/// resource: the archive downloaded into the installation, then unpacked.
async fn install(
    client: &Client,
    root: &Path,
    url: &str,
    archive: &Archive,
    cancel: &CancellationToken,
) -> Result<PathBuf, Error> {
    let unpacking = match install_archive(root, archive, cancel).await? {
        ArchiveInstall::Installed(entry) => return Ok(entry),
        ArchiveInstall::Unpack(unpacking) => unpacking,
    };
    let staging = root.join(format!(".download-{}", archive.digest.sha256));
    let received = download(client, plain(url), &staging, cancel, |_, _| {}).await?;
    let mut output = tokio::fs::File::create(unpacking.archive()).await?;
    received.verify(&staging, &archive.digest, &mut output, cancel).await?;
    drop(output);
    unpacking.finish(cancel).await
}

#[tokio::test]
async fn an_archive_is_installed_once_and_checked_before_each_use() {
    let client = download_client_allowing_http().unwrap();
    let cancel = CancellationToken::new();
    let bytes = zip(&[("app/bin/tool", b"tool"), ("app/data", b"data")]);
    let server = Server::start([("/app.zip".to_owned(), Answer::ok(bytes.clone()))]).await;
    let url = server.url("/app.zip");
    let archive = Archive {
        digest: declared(&bytes),
        entry: "app/bin/tool".to_owned(),
    };
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join(&archive.digest.sha256);
    assert_eq!(
        installed(&directory, &archive, &cancel).await.unwrap(),
        None
    );
    // Two installers at once: one downloads, the other finds its result.
    let (first, second) = tokio::join!(
        install(&client, root.path(), &url, &archive, &cancel),
        install(&client, root.path(), &url, &archive, &cancel),
    );
    let executable = first.unwrap();
    assert_eq!(second.unwrap(), executable);
    assert_eq!(executable, directory.join("app/bin/tool"));
    assert_eq!(std::fs::read(&executable).unwrap(), b"tool");
    assert_eq!(std::fs::read(directory.join("app/data")).unwrap(), b"data");
    assert_eq!(server.requests(), 1);
    assert_eq!(
        installed(&directory, &archive, &cancel).await.unwrap(),
        Some(executable.clone())
    );
    // Its installer's own cache trusts the receipt without reading the files.
    assert_eq!(
        recorded(&directory, &archive).await.unwrap(),
        Some(executable.clone())
    );
    // Nothing but the installation and its lock stays in the root.
    let lock = format!("{}.lock", archive.digest.sha256);
    assert_eq!(
        names(root.path()),
        [archive.digest.sha256.clone(), lock.clone()]
    );
    // A changed executable fails the check, and a new install neither
    // replaces it nor downloads again.
    std::fs::write(&executable, b"changed").unwrap();
    let result = installed(&directory, &archive, &cancel).await;
    assert!(
        matches!(result, Err(Error::Installation { .. })),
        "{result:?}"
    );
    let result = install(&client, root.path(), &url, &archive, &cancel).await;
    assert!(
        matches!(result, Err(Error::Installation { .. })),
        "{result:?}"
    );
    assert_eq!(server.requests(), 1);
    assert_eq!(
        recorded(&directory, &archive).await.unwrap(),
        Some(executable.clone())
    );
    std::fs::remove_file(directory.join(receipt::FILE)).unwrap();
    let result = installed(&directory, &archive, &cancel).await;
    assert!(
        matches!(result, Err(Error::Installation { .. })),
        "{result:?}"
    );
    // An archive that is not the declared one, or lacks its executable,
    // installs nothing.
    let other = tempfile::tempdir().unwrap();
    let wrong = Archive {
        digest: Digest {
            sha256: "0".repeat(64),
            ..archive.digest.clone()
        },
        ..archive.clone()
    };
    let result = install(&client, other.path(), &url, &wrong, &cancel).await;
    assert!(matches!(result, Err(Error::Digest)), "{result:?}");
    let lacking = Archive {
        entry: "app/bin/other".to_owned(),
        ..archive.clone()
    };
    let result = install(&client, other.path(), &url, &lacking, &cancel).await;
    assert!(matches!(result, Err(Error::Archive(_))), "{result:?}");
    let locks = [format!("{}.lock", "0".repeat(64)), lock];
    assert_eq!(names(other.path()), locks);
}

#[tokio::test]
async fn an_archive_extracts_inside_its_installation_only_and_names_its_files() {
    let client = download_client_allowing_http().unwrap();
    let cancel = CancellationToken::new();
    let escaping = zip(&[("app/bin/tool", b"tool"), ("../escaped", b"no")]);
    let server = Server::start([("/escaping.zip".to_owned(), Answer::ok(escaping.clone()))]).await;
    let archive = Archive {
        digest: declared(&escaping),
        entry: "app/bin/tool".to_owned(),
    };
    let root = tempfile::tempdir().unwrap();
    let installs = root.path().join("installs");
    let url = server.url("/escaping.zip");
    let result = install(&client, &installs, &url, &archive, &cancel).await;
    assert!(matches!(result, Err(Error::Archive(_))), "{result:?}");
    assert!(!root.path().join("escaped").exists());
    assert_eq!(
        names(&installs),
        [format!("{}.lock", archive.digest.sha256)]
    );
    let file = root.path().join("app.zip");
    std::fs::write(&file, zip(&[("app/bin/tool", b"tool")])).unwrap();
    assert!(holds(&file, "app/bin/tool").await.unwrap());
    assert!(!holds(&file, "app/bin").await.unwrap());
    assert!(!holds(&file, "app/bin/other").await.unwrap());
}

/// The Chrome runtime's archives are tar archives compressed with zstd
/// (`crates/command-package-browser-chrome/src/driver/installation.rs`):
/// one installs as a zip archive does, its empty directories with it, and
/// its entry is found the same way. Bytes of neither kind install nothing.
#[tokio::test]
async fn a_tar_archive_compressed_with_zstd_installs_as_a_zip_archive_does() {
    let client = download_client_allowing_http().unwrap();
    let cancel = CancellationToken::new();
    let bytes = tar_zst(&[
        ("lib/", b""),
        ("lib/gio/", b""),
        ("lib/gio/modules/", b""),
        ("./lib/libnss3.so", b"nss"),
    ]);
    let server = Server::start([
        ("/libs.tar.zst".to_owned(), Answer::ok(bytes.clone())),
        ("/plain".to_owned(), Answer::ok(b"plain bytes".to_vec())),
    ])
    .await;
    let archive = Archive {
        digest: declared(&bytes),
        entry: "lib/libnss3.so".to_owned(),
    };
    let root = tempfile::tempdir().unwrap();
    let entry = install(&client, root.path(), &server.url("/libs.tar.zst"), &archive, &cancel)
        .await
        .unwrap();
    let directory = root.path().join(&archive.digest.sha256);
    assert_eq!(entry, directory.join("lib/libnss3.so"));
    assert_eq!(std::fs::read(&entry).unwrap(), b"nss");
    assert!(directory.join("lib/gio/modules").is_dir());
    let file = root.path().join("libs.tar.zst");
    std::fs::write(&file, &bytes).unwrap();
    assert!(holds(&file, "lib/libnss3.so").await.unwrap());
    assert!(!holds(&file, "lib/gio/modules").await.unwrap());
    assert!(!holds(&file, "lib/libnspr4.so").await.unwrap());
    let plain = Archive {
        digest: declared(b"plain bytes"),
        entry: "lib/libnss3.so".to_owned(),
    };
    let other = tempfile::tempdir().unwrap();
    let result = install(&client, other.path(), &server.url("/plain"), &plain, &cancel).await;
    assert!(matches!(result, Err(Error::Archive(_))), "{result:?}");
}

/// A download whose digest the caller was given over a connection it trusts
/// follows a redirect, as GitHub serves a release's assets
/// (`crates/runner-command-packages/src/cache.rs`), and is verified all the same.
#[tokio::test]
async fn a_trusted_download_follows_a_redirect() {
    let client = download_client_allowing_http().unwrap();
    let cancel = CancellationToken::new();
    let storage = serve(200, BODY, true).await;
    let release = Server::start([(
        "/releases/download/1/libs.tar.zst".to_owned(),
        Answer::redirect(storage.url("/artifact")),
    )])
    .await;
    let url = release.url("/releases/download/1/libs.tar.zst");
    let request = Download {
        redirects: 10,
        ..plain(&url)
    };
    let output = fetch(&client, request, &declared(BODY), &cancel).await.unwrap();
    assert_eq!(output, BODY);
    assert_eq!(storage.requests(), 1);
}

#[test]
fn an_encoded_copy_checks_against_the_bytes_it_stands_for_only() {
    let bytes = BODY.repeat(100);
    let encoded = encode_blocking(&bytes, Effort::Fast).unwrap();
    check_encoded_blocking(&encoded, &declared(&bytes)).unwrap();
    let other = encode_blocking(b"other bytes", Effort::Fast).unwrap();
    assert!(check_encoded_blocking(&other, &declared(&bytes)).is_err());
    assert!(check_encoded_blocking(b"not zstd", &declared(&bytes)).is_err());
}

/// A fixture server on `127.0.0.1` that answers every connection with
/// `head` and `body` and then closes it, or, with no `head`, reads the
/// request and holds the connection without a byte.
async fn raw_server(head: Option<String>, body: &'static [u8]) -> (String, tokio::task::JoinHandle<()>) {
    use tokio::io::AsyncReadExt as _;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/artifact", listener.local_addr().unwrap());
    let serving = tokio::spawn(async move {
        let mut held = Vec::new();
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut request = vec![0; 4096];
            let _read = socket.read(&mut request).await;
            match &head {
                Some(head) => {
                    let _written = socket.write_all(head.as_bytes()).await;
                    let _written = socket.write_all(body).await;
                }
                None => held.push(socket),
            }
        }
    });
    (url, serving)
}

/// A connection whose body breaks off before a byte, until the download's
/// silence has passed, fails it with the cause beneath the layer that saw
/// it, not only reqwest's *error decoding response body*
/// (`crates/shared-artifacts/src/download.rs`). Cost: one local connection.
#[tokio::test]
async fn a_body_that_breaks_off_names_its_cause() {
    let head = "HTTP/1.1 200 OK\r\ncontent-length: 100\r\nconnection: close\r\n\r\n".to_owned();
    let (url, serving) = raw_server(Some(head), b"").await;
    let client = download_client_allowing_http().unwrap();
    let request = Download {
        patience: Patience {
            stall: Duration::from_millis(200),
            silence: Duration::from_millis(300),
        },
        ..plain(&url)
    };
    let directory = tempfile::tempdir().unwrap();
    let staging = directory.path().join("download");
    let result = download(&client, request, &staging, &CancellationToken::new(), |_, _| {}).await;
    let Err(Error::Download(message)) = result else {
        panic!("the download fails: {result:?}");
    };
    let mut layers = message.split(": ");
    let seen = layers.next().unwrap();
    assert!(layers.next().is_some(), "{seen} names no cause: {message}");
    serving.abort();
}

/// Connections that deliver nothing are given up once the download's
/// silence has passed, and the download fails saying so; one that stalls
/// after bytes would resume instead (`crates/shared-artifacts/src/download.rs`).
/// Cost: a third of a second of a test's patience.
#[tokio::test]
async fn connections_that_deliver_nothing_fail_the_download() {
    let (url, serving) = raw_server(None, b"").await;
    let client = download_client_allowing_http().unwrap();
    let request = Download {
        patience: Patience {
            stall: Duration::from_millis(200),
            silence: Duration::from_millis(300),
        },
        ..plain(&url)
    };
    let directory = tempfile::tempdir().unwrap();
    let staging = directory.path().join("download");
    let result = download(&client, request, &staging, &CancellationToken::new(), |_, _| {}).await;
    let Err(Error::Download(message)) = result else {
        panic!("the download fails: {result:?}");
    };
    assert!(message.starts_with("timed out"), "{message}");
    serving.abort();
}

/// A server that answers a resumed request with the whole file starts the
/// staging file again, and the download completes from it
/// (`crates/shared-artifacts/src/download.rs`). Cost: two local connections.
#[tokio::test]
async fn a_whole_answer_to_a_resumed_request_starts_the_file_again() {
    use tokio::io::AsyncReadExt as _;
    let body: Vec<u8> = (0u8..=255).cycle().take(64 * 1024).collect();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let serving = tokio::spawn({
        let body = body.clone();
        async move {
            let mut requests = Vec::new();
            for answer in 0..2 {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = vec![0; 4096];
                let read = socket.read(&mut request).await.unwrap();
                requests.push(String::from_utf8_lossy(&request[..read]).to_lowercase());
                let head = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", body.len());
                socket.write_all(head.as_bytes()).await.unwrap();
                // The first answer breaks off halfway; the second ignores the range.
                let sent = if answer == 0 { &body[..body.len() / 2] } else { &body[..] };
                socket.write_all(sent).await.unwrap();
            }
            requests
        }
    });
    let client = download_client_allowing_http().unwrap();
    let output = fetch(&client, plain(&url), &declared(&body), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(output, body);
    let requests = serving.await.unwrap();
    assert!(requests[1].contains("range: bytes="), "{}", requests[1]);
}

/// A local server that answers its connections in turn with `answers`,
/// each a head and a body, and then stops; it returns each request it read,
/// in lower case.
async fn scripted(answers: Vec<(String, Vec<u8>)>) -> (String, tokio::task::JoinHandle<Vec<String>>) {
    use tokio::io::AsyncReadExt as _;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/file", listener.local_addr().unwrap());
    let serving = tokio::spawn(async move {
        let mut requests = Vec::new();
        for (head, body) in answers {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 4096];
            let read = socket.read(&mut request).await.unwrap();
            requests.push(String::from_utf8_lossy(&request[..read]).to_lowercase());
            // The client may hang up first; nothing waits for the rest.
            let _written = socket.write_all(head.as_bytes()).await;
            let _written = socket.write_all(&body).await;
        }
        requests
    });
    (url, serving)
}

/// Bytes that do not compress, so a half of them is half the file.
fn noise(size: usize) -> Vec<u8> {
    (0u32..)
        .flat_map(|block| Sha256::digest(block.to_le_bytes()))
        .take(size)
        .collect()
}

/// A server that answers each resumed request with the whole file and
/// breaks before its end fails the download after the third time in a
/// row, and its staging file goes (`crates/shared-artifacts/src/download.rs`).
/// Cost: four local connections.
#[tokio::test]
async fn a_server_that_ignores_ranges_and_breaks_fails_after_three_times() {
    let body = noise(64 * 1024);
    let half = body[..body.len() / 2].to_vec();
    let head = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\n\r\n", body.len());
    let (url, serving) = scripted(vec![(head, half); 4]).await;
    let client = download_client_allowing_http().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let staging = directory.path().join("download");
    let result = tokio::time::timeout(
        Duration::from_secs(10),
        download(&client, plain(&url), &staging, &CancellationToken::new(), |_, _| {}),
    )
    .await
    .expect("the download gives up");
    assert!(
        matches!(&result, Err(Error::Download(message)) if message.contains("3 times")),
        "{result:?}"
    );
    assert_eq!(serving.await.unwrap().len(), 4);
    assert!(!staging.exists());
}

/// A download complete in its staging file but not yet checked, as when it
/// was cancelled before its check, is asked again, answered 416 without a
/// body or a coding, and checked in the coding its bytes arrived in
/// (`crates/shared-artifacts/src/download.rs`). Cost: two local connections.
#[tokio::test]
async fn a_complete_staging_file_keeps_its_coding_through_a_416() {
    let body = noise(16 * 1024);
    let encoded = encode_blocking(&body, Effort::Fast).unwrap();
    let whole = format!(
        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\ncontent-encoding: zstd\r\n\r\n",
        encoded.len()
    );
    let complete = format!(
        "HTTP/1.1 416 Range Not Satisfiable\r\ncontent-range: bytes */{}\r\ncontent-length: 0\r\n\r\n",
        encoded.len()
    );
    let (url, serving) = scripted(vec![(whole, encoded), (complete, Vec::new())]).await;
    let client = download_client_allowing_http().unwrap();
    let directory = tempfile::tempdir().unwrap();
    let staging = directory.path().join("download");
    let cancel = CancellationToken::new();
    // The first download ends without its check.
    download(&client, plain(&url), &staging, &cancel, |_, _| {}).await.unwrap();
    let received = download(&client, plain(&url), &staging, &cancel, |_, _| {}).await.unwrap();
    let mut output = Vec::new();
    received.verify(&staging, &declared(&body), &mut output, &cancel).await.unwrap();
    assert!(output == body, "the checked bytes differ");
    let requests = serving.await.unwrap();
    assert!(requests[1].contains("range: bytes="), "{}", requests[1]);
}

/// A resumed answer whose range does not start where the staging file ends
/// starts the file again (`crates/shared-artifacts/src/download.rs`). Cost: three
/// local connections.
#[tokio::test]
async fn a_range_that_does_not_continue_the_file_starts_it_again() {
    let body = noise(64 * 1024);
    let half = body.len() / 2;
    let size = body.len();
    let broken = (
        format!("HTTP/1.1 200 OK\r\ncontent-length: {size}\r\n\r\n"),
        body[..half].to_vec(),
    );
    let elsewhere = (
        format!(
            "HTTP/1.1 206 Partial Content\r\ncontent-range: bytes 10-{}/{size}\r\ncontent-length: {}\r\n\r\n",
            size - 1,
            size - 10
        ),
        body[10..].to_vec(),
    );
    let whole = (
        format!("HTTP/1.1 200 OK\r\ncontent-length: {size}\r\nconnection: close\r\n\r\n"),
        body.clone(),
    );
    let (url, serving) = scripted(vec![broken, elsewhere, whole]).await;
    let client = download_client_allowing_http().unwrap();
    let output = fetch(&client, plain(&url), &declared(&body), &CancellationToken::new())
        .await
        .unwrap();
    assert!(output == body, "the downloaded bytes differ");
    let requests = serving.await.unwrap();
    assert!(!requests[2].contains("range:"), "{}", requests[2]);
}
