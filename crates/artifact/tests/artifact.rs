//! Verified downloads and copies, digests, publication, the install lock,
//! receipts and zip extraction.

use std::{io::Write as _, path::Path, time::Duration};

use demi_artifact::{
    Digest, Error, InstallLock, Mode, Permissions, Publication, Staged, Verifier, copy, digest,
    download, extract_zip, publish, publish_bytes, publish_directory, receipt,
    testing::loopback_client,
};
use sha2::{Digest as _, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

const BODY: &[u8] = b"verified bytes";

fn declared(bytes: &[u8]) -> Digest {
    Digest {
        size: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
    }
}

/// A local HTTP server that answers every request with `status`, `body` and
/// an optional Content-Length.
async fn serve(status: &'static str, body: &'static [u8], length: Option<usize>) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut request = vec![0; 4096];
                let _read = socket.read(&mut request).await;
                let length = length.map_or(String::new(), |length| format!("content-length: {length}\r\n"));
                let head = format!("HTTP/1.1 {status}\r\n{length}connection: close\r\n\r\n");
                let _written = socket.write_all(head.as_bytes()).await;
                let _written = socket.write_all(body).await;
            });
        }
    });
    format!("http://{address}/artifact")
}

#[tokio::test]
async fn a_download_is_verified_as_it_arrives() {
    let client = loopback_client().unwrap();
    let cancel = CancellationToken::new();
    let url = serve("200 OK", BODY, Some(BODY.len())).await;
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
    let unsized_url = serve("200 OK", BODY, None).await;
    let result = download(&client, &unsized_url, &short, &mut Vec::new(), &cancel).await;
    assert!(matches!(result, Err(Error::TooLarge { declared: 3 })), "{result:?}");
    let missing = serve("404 Not Found", b"", Some(0)).await;
    let result = download(&client, &missing, &declared(BODY), &mut Vec::new(), &cancel).await;
    assert!(matches!(result, Err(Error::Rejected { status: 404 })), "{result:?}");
    let moved = serve("302 Found", b"", Some(0)).await;
    let result = download(&client, &moved, &declared(BODY), &mut Vec::new(), &cancel).await;
    assert!(matches!(result, Err(Error::Rejected { status: 302 })), "{result:?}");
    cancel.cancel();
    let result = download(&client, &url, &declared(BODY), &mut Vec::new(), &cancel).await;
    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
}

#[tokio::test]
async fn the_download_client_refuses_plain_http() {
    let client = demi_artifact::client().unwrap();
    let url = serve("200 OK", BODY, Some(BODY.len())).await;
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

fn archive(entries: &[(&str, &[u8])]) -> tempfile::NamedTempFile {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, contents) in entries {
        writer
            .start_file(*name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(contents).unwrap();
    }
    let bytes = writer.finish().unwrap().into_inner();
    file.write_all(&bytes).unwrap();
    file
}

#[tokio::test]
async fn a_zip_extracts_inside_its_destination_only() {
    let root = tempfile::tempdir().unwrap();
    let cancel = CancellationToken::new();
    let good = archive(&[("app/bin/tool", b"tool")]);
    let destination = root.path().join("extracted");
    extract_zip(good.path(), &destination, &cancel).await.unwrap();
    assert_eq!(std::fs::read(destination.join("app/bin/tool")).unwrap(), b"tool");
    let escaping = archive(&[("../escaped", b"no")]);
    let result = extract_zip(escaping.path(), &root.path().join("other"), &cancel).await;
    assert!(matches!(result, Err(Error::Archive(_))), "{result:?}");
    assert!(!root.path().join("escaped").exists());
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let result = extract_zip(good.path(), &root.path().join("cancelled"), &cancelled).await;
    assert!(matches!(result, Err(Error::Cancelled)), "{result:?}");
}
