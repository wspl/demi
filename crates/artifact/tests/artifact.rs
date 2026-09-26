//! Verified and measured downloads, copies, digests, publication, release
//! publication, the install lock, receipts and archive installation.

use std::{path::Path, time::Duration};

use demi_artifact::{
    Archive, Digest, Error, InstallLock, Mode, Permissions, Publication, ReleaseFile, ReleaseRecord,
    Staged, Verifier, copy, digest, download, download_measured, install_archive, installed, publish,
    publish_bytes, publish_directory, publish_release, receipt,
    testing::{Answer, Server, loopback_client, zip},
    zip_holds,
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
        body: body.to_vec(),
        length,
    };
    Server::start([("/artifact".to_owned(), answer)]).await
}

#[tokio::test]
async fn a_download_is_verified_as_it_arrives() {
    let client = loopback_client().unwrap();
    let cancel = CancellationToken::new();
    let server = serve(200, BODY, true).await;
    let url = server.url("/artifact");
    let mut output = Vec::new();
    download(&client, &url, &declared(BODY), &mut output, &cancel).await.unwrap();
    assert_eq!(output, BODY);
    let wrong = Digest {
        sha256: "0".repeat(64),
        ..declared(BODY)
    };
    let result = download(&client, &url, &wrong, &mut Vec::new(), &cancel).await;
    assert!(matches!(result, Err(Error::Digest)), "{result:?}");
    // A declared size the server's length contradicts fails before the body.
    let short = Digest {
        size: 3,
        ..declared(BODY)
    };
    let result = download(&client, &url, &short, &mut Vec::new(), &cancel).await;
    assert!(matches!(result, Err(Error::Size { declared: 3, .. })), "{result:?}");
    // Without a length, bytes past the declared size stop the download.
    let unsized_server = serve(200, BODY, false).await;
    let unsized_url = unsized_server.url("/artifact");
    let result = download(&client, &unsized_url, &short, &mut Vec::new(), &cancel).await;
    assert!(matches!(result, Err(Error::TooLarge { declared: 3 })), "{result:?}");
    let missing = server.url("/elsewhere");
    let result = download(&client, &missing, &declared(BODY), &mut Vec::new(), &cancel).await;
    assert!(matches!(result, Err(Error::Rejected { status: 404 })), "{result:?}");
    let moved = serve(302, b"", true).await;
    let result = download(&client, &moved.url("/artifact"), &declared(BODY), &mut Vec::new(), &cancel).await;
    assert!(matches!(result, Err(Error::Rejected { status: 302 })), "{result:?}");
    cancel.cancel();
    let result = download(&client, &url, &declared(BODY), &mut Vec::new(), &cancel).await;
    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
}

#[tokio::test]
async fn a_measured_download_reports_what_arrived_within_its_limit() {
    let client = loopback_client().unwrap();
    let cancel = CancellationToken::new();
    let server = serve(200, BODY, true).await;
    let mut output = Vec::new();
    let measured = download_measured(&client, &server.url("/artifact"), 1024, &mut output, &cancel)
        .await
        .unwrap();
    assert_eq!((output.as_slice(), measured), (BODY, declared(BODY)));
    // A declared length past the limit fails before the body, and without
    // one, bytes past the limit stop the download.
    let result = download_measured(&client, &server.url("/artifact"), 3, &mut Vec::new(), &cancel).await;
    assert!(matches!(result, Err(Error::TooLarge { declared: 3 })), "{result:?}");
    let unsized_server = serve(200, BODY, false).await;
    let result = download_measured(&client, &unsized_server.url("/artifact"), 3, &mut Vec::new(), &cancel).await;
    assert!(matches!(result, Err(Error::TooLarge { declared: 3 })), "{result:?}");
    let result = download_measured(&client, &server.url("/elsewhere"), 1024, &mut Vec::new(), &cancel).await;
    assert!(matches!(result, Err(Error::Rejected { status: 404 })), "{result:?}");
}

#[tokio::test]
async fn the_download_client_refuses_plain_http() {
    let client = demi_artifact::client().unwrap();
    let server = serve(200, BODY, true).await;
    let url = server.url("/artifact");
    let result = download(&client, &url, &declared(BODY), &mut Vec::new(), &CancellationToken::new()).await;
    assert!(matches!(result, Err(Error::Download(_))), "{result:?}");
}

#[tokio::test]
async fn copies_and_digests_check_the_declared_bytes() {
    let directory = tempfile::tempdir().unwrap();
    let source = directory.path().join("source");
    std::fs::write(&source, BODY).unwrap();
    let cancel = CancellationToken::new();
    assert_eq!(digest(&source, 1024, &cancel).await.unwrap(), declared(BODY));
    assert!(matches!(digest(&source, 3, &cancel).await, Err(Error::TooLarge { declared: 3 })));
    let mut output = Vec::new();
    let mut input = tokio::fs::File::open(&source).await.unwrap();
    copy(&mut input, &declared(BODY), &mut output, &cancel).await.unwrap();
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
    assert!(matches!(digest(&source, 1024, &cancel).await, Err(Error::Cancelled)));
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
    publish_bytes(&path, b"first", publication(Mode::CreateNew, Permissions::Default))
        .await
        .unwrap();
    let again = publish_bytes(&path, b"second", publication(Mode::CreateNew, Permissions::Default)).await;
    assert!(matches!(&again, Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::AlreadyExists), "{again:?}");
    assert_eq!(std::fs::read(&path).unwrap(), b"first");
    let mut input: &[u8] = b"replaced";
    publish(&path, &mut input, publication(Mode::Replace, Permissions::Default), &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), b"replaced");
    // Nothing but the file remains beside it.
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    // A cancelled publication leaves the old file and no temporary.
    let cancel = CancellationToken::new();
    cancel.cancel();
    let mut input: &[u8] = b"never";
    let result = publish(&path, &mut input, publication(Mode::Replace, Permissions::Default), &cancel).await;
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
    drop(abandoned);
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 0);
    let mut staged = Staged::new(&path, publication(Mode::CreateNew, Permissions::Executable))
        .await
        .unwrap();
    let mut input = tokio::fs::File::open(env!("CARGO_MANIFEST_DIR").to_owned() + "/Cargo.toml")
        .await
        .unwrap();
    let expected = std::fs::read(env!("CARGO_MANIFEST_DIR").to_owned() + "/Cargo.toml").unwrap();
    copy(&mut input, &declared(&expected), staged.file(), &CancellationToken::new())
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
    publish_bytes(&private, b"secret", publication(Mode::CreateNew, Permissions::Private))
        .await
        .unwrap();
    assert_eq!(mode(&private), 0o600);
    let executable = directory.path().join("tool");
    publish_bytes(&executable, b"#!/bin/sh\n", publication(Mode::CreateNew, Permissions::Executable))
        .await
        .unwrap();
    assert_eq!(mode(&executable), 0o755);
    std::fs::set_permissions(&private, std::fs::Permissions::from_mode(0o640)).unwrap();
    publish_bytes(&private, b"rotated", publication(Mode::Replace, Permissions::Keep))
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
    publish_release(&directory, record, &files, &cancel).await.unwrap();
    assert_eq!(std::fs::read(directory.join("descriptor.json")).unwrap(), record.bytes);
    assert_eq!(std::fs::read(directory.join("x86_64-unknown-linux-musl/tool")).unwrap(), b"tool v1");
    assert_eq!(std::fs::read(directory.join("data")).unwrap(), b"data");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o111;
        assert_eq!(mode(&directory.join("x86_64-unknown-linux-musl/tool")), 0o111);
        assert_eq!(mode(&directory.join("data")), 0);
    }
    // The same release again is the one in place.
    publish_release(&directory, record, &files, &cancel).await.unwrap();
    // Another record under the same name is refused, and so is a file that
    // changed in place; nothing in place changes.
    let other = ReleaseRecord {
        name: "descriptor.json",
        bytes: b"{\"version\":\"2\"}\n",
    };
    let refused = publish_release(&directory, other, &files, &cancel).await;
    assert!(matches!(&refused, Err(Error::Conflict(path)) if path.ends_with("descriptor.json")), "{refused:?}");
    std::fs::write(directory.join("data"), b"corrupt").unwrap();
    let refused = publish_release(&directory, record, &files, &cancel).await;
    assert!(matches!(&refused, Err(Error::Conflict(path)) if path.ends_with("data")), "{refused:?}");
    assert_eq!(std::fs::read(directory.join("descriptor.json")).unwrap(), record.bytes);
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

#[tokio::test]
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
    assert!(matches!(quitter.await.unwrap(), Err(Error::Cancelled)));
    drop(next);
}

#[tokio::test]
async fn receipts_round_trip_and_an_absent_one_is_none() {
    let directory = tempfile::tempdir().unwrap();
    assert_eq!(receipt::read(directory.path()).await.unwrap(), None);
    receipt::write(directory.path(), &serde_json::json!({"sha256": "a"})).await.unwrap();
    let bytes = receipt::read(directory.path()).await.unwrap().unwrap();
    assert_eq!(serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()["sha256"], "a");
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

#[tokio::test]
async fn an_archive_is_installed_once_and_checked_before_each_use() {
    let client = loopback_client().unwrap();
    let cancel = CancellationToken::new();
    let bytes = zip(&[("app/bin/tool", b"tool"), ("app/data", b"data")]);
    let server = Server::start([("/app.zip".to_owned(), Answer::ok(bytes.clone()))]).await;
    let archive = Archive {
        url: server.url("/app.zip"),
        digest: declared(&bytes),
        executable: "app/bin/tool".to_owned(),
    };
    let root = tempfile::tempdir().unwrap();
    let directory = root.path().join(&archive.digest.sha256);
    assert_eq!(installed(&directory, &archive, &cancel).await.unwrap(), None);
    // Two installers at once: one downloads, the other finds its result.
    let (first, second) = tokio::join!(
        install_archive(&client, root.path(), &archive, &cancel),
        install_archive(&client, root.path(), &archive, &cancel),
    );
    let executable = first.unwrap();
    assert_eq!(second.unwrap(), executable);
    assert_eq!(executable, directory.join("app/bin/tool"));
    assert_eq!(std::fs::read(&executable).unwrap(), b"tool");
    assert_eq!(std::fs::read(directory.join("app/data")).unwrap(), b"data");
    assert_eq!(server.requests(), 1);
    assert_eq!(installed(&directory, &archive, &cancel).await.unwrap(), Some(executable.clone()));
    // Nothing but the installation and its lock stays in the root.
    let lock = format!("{}.lock", archive.digest.sha256);
    assert_eq!(names(root.path()), [archive.digest.sha256.clone(), lock.clone()]);
    // A changed executable fails the check, and a new install neither
    // replaces it nor downloads again.
    std::fs::write(&executable, b"changed").unwrap();
    let result = installed(&directory, &archive, &cancel).await;
    assert!(matches!(result, Err(Error::Installation { .. })), "{result:?}");
    let result = install_archive(&client, root.path(), &archive, &cancel).await;
    assert!(matches!(result, Err(Error::Installation { .. })), "{result:?}");
    assert_eq!(server.requests(), 1);
    std::fs::remove_file(directory.join(receipt::FILE)).unwrap();
    let result = installed(&directory, &archive, &cancel).await;
    assert!(matches!(result, Err(Error::Installation { .. })), "{result:?}");
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
    let result = install_archive(&client, other.path(), &wrong, &cancel).await;
    assert!(matches!(result, Err(Error::Digest)), "{result:?}");
    let lacking = Archive {
        executable: "app/bin/other".to_owned(),
        ..archive.clone()
    };
    let result = install_archive(&client, other.path(), &lacking, &cancel).await;
    assert!(matches!(result, Err(Error::Archive(_))), "{result:?}");
    let locks = [format!("{}.lock", "0".repeat(64)), lock];
    assert_eq!(names(other.path()), locks);
}

#[tokio::test]
async fn an_archive_extracts_inside_its_installation_only_and_names_its_files() {
    let client = loopback_client().unwrap();
    let cancel = CancellationToken::new();
    let escaping = zip(&[("app/bin/tool", b"tool"), ("../escaped", b"no")]);
    let server = Server::start([("/escaping.zip".to_owned(), Answer::ok(escaping.clone()))]).await;
    let archive = Archive {
        url: server.url("/escaping.zip"),
        digest: declared(&escaping),
        executable: "app/bin/tool".to_owned(),
    };
    let root = tempfile::tempdir().unwrap();
    let installs = root.path().join("installs");
    let result = install_archive(&client, &installs, &archive, &cancel).await;
    assert!(matches!(result, Err(Error::Archive(_))), "{result:?}");
    assert!(!root.path().join("escaped").exists());
    assert_eq!(names(&installs), [format!("{}.lock", archive.digest.sha256)]);
    let file = root.path().join("app.zip");
    std::fs::write(&file, zip(&[("app/bin/tool", b"tool")])).unwrap();
    assert!(zip_holds(&file, "app/bin/tool").await.unwrap());
    assert!(!zip_holds(&file, "app/bin").await.unwrap());
    assert!(!zip_holds(&file, "app/bin/other").await.unwrap());
}
