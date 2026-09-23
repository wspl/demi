use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use bytes::Bytes;
use demi_claude_protocol::Installed;
use demi_command_service::{
    Handler, Input, InvocationContext, Output,
    protocol::{CommandCaller, CommandContext, CommandLocale, Completion, Invocation, Record},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio_util::sync::CancellationToken;

use crate::{
    DemiClaude,
    install::{EnsureError, Installer, Roots},
    platform::{self, Loaders, platform_key},
    version,
};

const BINARY: &str = if cfg!(windows) {
    "claude.exe"
} else {
    "claude"
};

/// A local HTTP server that answers every request with the same bytes.
struct Fixture {
    address: SocketAddr,
    requests: Arc<AtomicUsize>,
}

impl Fixture {
    async fn serve(body: &'static [u8], delay: Duration) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();
        tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let counter = counter.clone();
                tokio::spawn(async move {
                    let mut request = Vec::new();
                    let mut buffer = [0; 1024];
                    while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                        match socket.read(&mut buffer).await {
                            Ok(count) if count > 0 => request.extend_from_slice(&buffer[..count]),
                            _ => return,
                        }
                    }
                    counter.fetch_add(1, Ordering::SeqCst);
                    tokio::time::sleep(delay).await;
                    let head = format!(
                        "HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                        body.len()
                    );
                    let _closed = socket.write_all(head.as_bytes()).await;
                    let _closed = socket.write_all(body).await;
                });
            }
        });
        Self { address, requests }
    }

    fn requests(&self) -> usize {
        self.requests.load(Ordering::SeqCst)
    }

    /// A release record whose entry for this machine points at this server.
    fn record(&self, version: &str, size: usize, sha256: &str) -> Vec<u8> {
        record(
            version,
            platform::current().unwrap(),
            &format!("http://{}/{version}/claude", self.address),
            size,
            sha256,
        )
    }
}

fn record(version: &str, platform: &str, url: &str, size: usize, sha256: &str) -> Vec<u8> {
    json!({ "version": version, "platforms": { platform: { "url": url, "size": size, "sha256": sha256 } } })
        .to_string()
        .into_bytes()
}

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

struct Machine {
    _directory: tempfile::TempDir,
    home: PathBuf,
    image: PathBuf,
}

impl Machine {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let home = directory.path().join("home/.demi/claude");
        let image = directory.path().join("opt/demi/claude");
        Self {
            _directory: directory,
            home,
            image,
        }
    }

    fn installer(&self) -> Installer {
        Installer::with_loopback_http(Roots {
            home: Some(self.home.clone()),
            image: Some(self.image.clone()),
        })
    }

    /// The version directories and staging directories left under the user's root.
    fn home_directories(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(&self.home) else {
            return Vec::new();
        };
        let mut names: Vec<String> = entries
            .map(|entry| entry.unwrap())
            .filter(|entry| entry.file_type().unwrap().is_dir())
            .map(|entry| entry.file_name().into_string().unwrap())
            .collect();
        names.sort();
        names
    }
}

fn preinstall(root: &Path, version: &str, body: &[u8]) {
    let directory = root.join(version);
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join(BINARY), body).unwrap();
    let receipt = json!({
        "version": version,
        "platform": platform::current().unwrap(),
        "sha256": sha256(body),
        "size": body.len(),
    });
    std::fs::write(directory.join("receipt.json"), receipt.to_string()).unwrap();
}

async fn ensure(installer: &Installer, record: &[u8]) -> Result<Installed, EnsureError> {
    let release = installer.release(record)?;
    installer.ensure(&release, &CancellationToken::new()).await
}

const BODY: &[u8] = b"#!/bin/sh\necho fixture claude\n";

#[tokio::test]
async fn ensure_installs_the_executable_and_its_receipt() {
    let machine = Machine::new();
    let fixture = Fixture::serve(BODY, Duration::ZERO).await;
    let installer = machine.installer();
    let installed = ensure(
        &installer,
        &fixture.record("2.1.278", BODY.len(), &sha256(BODY)),
    )
    .await
    .unwrap();
    assert_eq!(installed.version, "2.1.278");
    assert_eq!(installed.path, machine.home.join("2.1.278").join(BINARY));
    assert!(installed.path.is_absolute());
    assert_eq!(std::fs::read(&installed.path).unwrap(), BODY);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&installed.path)
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111);
    }
    let receipt: Value =
        serde_json::from_slice(&std::fs::read(machine.home.join("2.1.278/receipt.json")).unwrap())
            .unwrap();
    assert_eq!(
        receipt,
        json!({
            "version": "2.1.278",
            "platform": platform::current().unwrap(),
            "sha256": sha256(BODY),
            "size": BODY.len(),
        })
    );
    assert_eq!(machine.home_directories(), ["2.1.278"]);
}

#[tokio::test]
async fn a_second_ensure_does_not_download() {
    let machine = Machine::new();
    let fixture = Fixture::serve(BODY, Duration::ZERO).await;
    let record = fixture.record("2.1.278", BODY.len(), &sha256(BODY));
    let installer = machine.installer();
    let first = ensure(&installer, &record).await.unwrap();
    let second = ensure(&installer, &record).await.unwrap();
    // Another service process verifies the installation rather than trusting memory.
    let third = ensure(&machine.installer(), &record).await.unwrap();
    assert_eq!(first, second);
    assert_eq!(first, third);
    assert_eq!(fixture.requests(), 1);
}

#[tokio::test]
async fn a_changed_installation_is_replaced() {
    let machine = Machine::new();
    let fixture = Fixture::serve(BODY, Duration::ZERO).await;
    let record = fixture.record("2.1.278", BODY.len(), &sha256(BODY));
    let installed = ensure(&machine.installer(), &record).await.unwrap();
    let mut changed = BODY.to_vec();
    changed[0] ^= 1;
    std::fs::write(&installed.path, changed).unwrap();
    ensure(&machine.installer(), &record).await.unwrap();
    assert_eq!(std::fs::read(&installed.path).unwrap(), BODY);
    assert_eq!(fixture.requests(), 2);
}

#[tokio::test]
async fn a_wrong_digest_installs_nothing() {
    let machine = Machine::new();
    let fixture = Fixture::serve(BODY, Duration::ZERO).await;
    let record = fixture.record("2.1.278", BODY.len(), &sha256(b"another executable"));
    let error = ensure(&machine.installer(), &record).await.unwrap_err();
    assert_eq!(error.code().map(|code| code.to_string()).as_deref(), Some("verification_failed"));
    assert!(error.to_string().contains("2.1.278"), "{error}");
    assert!(error.to_string().contains("127.0.0.1"), "{error}");
    assert!(machine.home_directories().is_empty());
}

#[tokio::test]
async fn a_body_longer_than_its_size_installs_nothing() {
    let machine = Machine::new();
    let fixture = Fixture::serve(BODY, Duration::ZERO).await;
    let declared = &BODY[..BODY.len() - 1];
    let record = fixture.record("2.1.278", declared.len(), &sha256(declared));
    let error = ensure(&machine.installer(), &record).await.unwrap_err();
    assert_eq!(error.code().map(|code| code.to_string()).as_deref(), Some("verification_failed"));
    assert!(machine.home_directories().is_empty());
}

#[tokio::test]
async fn a_body_shorter_than_its_size_installs_nothing() {
    let machine = Machine::new();
    let fixture = Fixture::serve(BODY, Duration::ZERO).await;
    let record = fixture.record("2.1.278", BODY.len() + 1, &sha256(BODY));
    let error = ensure(&machine.installer(), &record).await.unwrap_err();
    assert_eq!(error.code().map(|code| code.to_string()).as_deref(), Some("verification_failed"));
    assert!(machine.home_directories().is_empty());
}

#[tokio::test]
async fn a_record_without_this_platform_is_unsupported() {
    let machine = Machine::new();
    let fixture = Fixture::serve(BODY, Duration::ZERO).await;
    let record = record(
        "2.1.278",
        "plan9-mips",
        &format!("http://{}/claude", fixture.address),
        BODY.len(),
        &sha256(BODY),
    );
    let error = ensure(&machine.installer(), &record).await.unwrap_err();
    assert_eq!(error.code().map(|code| code.to_string()).as_deref(), Some("unsupported_platform"));
    assert!(
        error.to_string().contains(platform::current().unwrap()),
        "{error}"
    );
    assert_eq!(fixture.requests(), 0);
}

#[tokio::test]
async fn a_new_version_removes_the_old_one() {
    let machine = Machine::new();
    let fixture = Fixture::serve(BODY, Duration::ZERO).await;
    let installer = machine.installer();
    ensure(
        &installer,
        &fixture.record("2.1.277", BODY.len(), &sha256(BODY)),
    )
    .await
    .unwrap();
    ensure(
        &installer,
        &fixture.record("2.1.278", BODY.len(), &sha256(BODY)),
    )
    .await
    .unwrap();
    assert_eq!(machine.home_directories(), ["2.1.278"]);
}

#[tokio::test]
async fn a_preinstalled_version_is_used_without_downloading() {
    let machine = Machine::new();
    let fixture = Fixture::serve(BODY, Duration::ZERO).await;
    preinstall(&machine.image, "2.1.278", BODY);
    let installed = ensure(
        &machine.installer(),
        &fixture.record("2.1.278", BODY.len(), &sha256(BODY)),
    )
    .await
    .unwrap();
    assert_eq!(installed.path, machine.image.join("2.1.278").join(BINARY));
    assert_eq!(fixture.requests(), 0);
    assert!(machine.home_directories().is_empty());
}

#[tokio::test]
async fn a_mismatching_preinstalled_version_is_ignored_and_kept() {
    let machine = Machine::new();
    let fixture = Fixture::serve(BODY, Duration::ZERO).await;
    preinstall(
        &machine.image,
        "2.1.278",
        b"an older build of the same version",
    );
    let installed = ensure(
        &machine.installer(),
        &fixture.record("2.1.278", BODY.len(), &sha256(BODY)),
    )
    .await
    .unwrap();
    assert_eq!(installed.path, machine.home.join("2.1.278").join(BINARY));
    assert_eq!(
        std::fs::read(machine.image.join("2.1.278").join(BINARY)).unwrap(),
        b"an older build of the same version"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn concurrent_ensures_download_once() {
    let machine = Machine::new();
    let fixture = Fixture::serve(BODY, Duration::from_millis(200)).await;
    let record = fixture.record("2.1.278", BODY.len(), &sha256(BODY));
    // One installer is two invocations of a service; the other is another process.
    let (service, process) = (machine.installer(), machine.installer());
    let (first, second, third) = tokio::join!(
        ensure(&service, &record),
        ensure(&service, &record),
        ensure(&process, &record),
    );
    assert_eq!(first.unwrap(), second.unwrap());
    assert_eq!(third.unwrap().version, "2.1.278");
    assert_eq!(fixture.requests(), 1);
    assert_eq!(machine.home_directories(), ["2.1.278"]);
}

#[tokio::test]
async fn cancellation_stops_a_download_and_installs_nothing() {
    let machine = Machine::new();
    let fixture = Fixture::serve(BODY, Duration::from_secs(30)).await;
    let installer = machine.installer();
    let release = installer
        .release(&fixture.record("2.1.278", BODY.len(), &sha256(BODY)))
        .unwrap();
    let cancel = CancellationToken::new();
    let canceller = cancel.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        canceller.cancel();
    });
    let error = installer.ensure(&release, &cancel).await.unwrap_err();
    assert!(matches!(error, EnsureError::Cancelled));
    assert!(machine.home_directories().is_empty());
}

#[test]
fn malformed_records_are_invalid() {
    let installer = Installer::default();
    let digest = sha256(BODY);
    let artifact =
        json!({ "url": "https://downloads.claude.ai/claude", "size": 1, "sha256": digest });
    let invalid = [
        json!("not a record"),
        json!({ "version": "2.1.278" }),
        json!({ "version": "2.1.278", "platforms": {}, "extra": true }),
        json!({ "version": "2.1.278", "platforms": { "linux-x64": { "url": "https://downloads.claude.ai/claude", "size": 1, "sha256": digest, "extra": 1 } } }),
        json!({ "version": "2.1.278", "platforms": { "linux-x64": { "url": "http://downloads.claude.ai/claude", "size": 1, "sha256": digest } } }),
        json!({ "version": "2.1.278", "platforms": { "linux-x64": { "url": "http://127.0.0.1/claude", "size": 1, "sha256": digest } } }),
        json!({ "version": "2.1.278", "platforms": { "linux-x64": { "url": "https://downloads.claude.ai/claude", "size": 0, "sha256": digest } } }),
        json!({ "version": "2.1.278", "platforms": { "linux-x64": { "url": "https://downloads.claude.ai/claude", "size": 1, "sha256": "abc" } } }),
        json!({ "version": "2.1.278", "platforms": { "linux-x64": { "url": "https://downloads.claude.ai/claude", "size": 1, "sha256": digest.to_uppercase() } } }),
    ];
    for record in invalid {
        let error = installer
            .release(record.to_string().as_bytes())
            .unwrap_err();
        assert_eq!(error.code().map(|code| code.to_string()).as_deref(), Some("invalid_release"), "{record}");
    }
    for version in [
        "",
        "2.1",
        "2.1.278.1",
        "v2.1.278",
        "2.1.x",
        "../2.1.278",
        "2.1.278/..",
        "2.1.278-",
        "2.1.278-a/b",
        "2.1.278+build",
        " 2.1.278",
    ] {
        let record = json!({ "version": version, "platforms": { "linux-x64": artifact } });
        let error = installer
            .release(record.to_string().as_bytes())
            .unwrap_err();
        assert_eq!(error.code().map(|code| code.to_string()).as_deref(), Some("invalid_release"), "{version:?}");
    }
    assert!(installer.release(b"{").is_err());
    for version in ["2.1.278", "0.0.0", "10.20.30-beta.1", "1.0.0-rc-1"] {
        let record = json!({ "version": version, "platforms": { "linux-x64": artifact } });
        assert!(
            installer.release(record.to_string().as_bytes()).is_ok(),
            "{version}"
        );
    }
}

#[test]
fn the_platform_key_follows_the_machine() {
    let none = Loaders::default();
    let glibc = Loaders {
        musl: false,
        glibc: true,
    };
    let musl = Loaders {
        musl: true,
        glibc: false,
    };
    let both = Loaders {
        musl: true,
        glibc: true,
    };
    assert_eq!(platform_key("macos", "aarch64", none), Some("darwin-arm64"));
    assert_eq!(platform_key("macos", "x86_64", none), Some("darwin-x64"));
    assert_eq!(platform_key("windows", "x86_64", none), Some("win32-x64"));
    assert_eq!(
        platform_key("windows", "aarch64", none),
        Some("win32-arm64")
    );
    assert_eq!(platform_key("linux", "x86_64", glibc), Some("linux-x64"));
    assert_eq!(
        platform_key("linux", "x86_64", musl),
        Some("linux-x64-musl")
    );
    assert_eq!(platform_key("linux", "x86_64", both), Some("linux-x64"));
    assert_eq!(platform_key("linux", "x86_64", none), Some("linux-x64"));
    assert_eq!(platform_key("linux", "aarch64", glibc), Some("linux-arm64"));
    assert_eq!(
        platform_key("linux", "aarch64", musl),
        Some("linux-arm64-musl")
    );
    assert_eq!(platform_key("linux", "aarch64", both), Some("linux-arm64"));
    assert_eq!(platform_key("linux", "aarch64", none), Some("linux-arm64"));
    assert_eq!(platform_key("macos", "arm", musl), None);
    assert_eq!(platform_key("linux", "riscv64", glibc), None);
    assert_eq!(platform_key("freebsd", "x86_64", none), None);
}

#[test]
fn versions_order_semantically() {
    let ascending = [
        "1.0.0-alpha",
        "1.0.0-alpha.1",
        "1.0.0-alpha.beta",
        "1.0.0-beta.2",
        "1.0.0-beta.11",
        "1.0.0",
        "2.1.9",
        "2.1.10",
        "2.1.278",
        "2.10.0",
        "10.0.0",
    ];
    for (index, left) in ascending.iter().enumerate() {
        for (other, right) in ascending.iter().enumerate() {
            assert_eq!(
                version::compare(left, right),
                index.cmp(&other),
                "{left} {right}"
            );
        }
    }
}

#[tokio::test]
async fn status_lists_installations_newest_first() {
    let machine = Machine::new();
    preinstall(&machine.home, "2.1.9", BODY);
    preinstall(&machine.home, "2.1.278", BODY);
    preinstall(&machine.home, "2.1.278-beta.1", BODY);
    preinstall(&machine.image, "2.1.10", BODY);
    // Not installations: no receipt, no executable, a receipt of another version, a stray name.
    std::fs::create_dir_all(machine.home.join("3.0.0")).unwrap();
    std::fs::write(machine.home.join("3.0.0").join(BINARY), BODY).unwrap();
    preinstall(&machine.home, "3.0.1", BODY);
    std::fs::remove_file(machine.home.join("3.0.1").join(BINARY)).unwrap();
    preinstall(&machine.home, "3.0.2", BODY);
    std::fs::rename(machine.home.join("3.0.2"), machine.home.join("3.0.3")).unwrap();
    preinstall(&machine.home, "install-abc", BODY);
    std::fs::write(machine.home.join("2.1.278.lock"), b"").unwrap();

    let status = machine.installer().status().await.unwrap();
    assert_eq!(status.platform, platform::current().unwrap());
    let installed: Vec<(&str, PathBuf)> = status
        .installed
        .iter()
        .map(|installed| (installed.version.as_str(), installed.path.clone()))
        .collect();
    assert_eq!(
        installed,
        [
            ("2.1.278", machine.home.join("2.1.278").join(BINARY)),
            (
                "2.1.278-beta.1",
                machine.home.join("2.1.278-beta.1").join(BINARY)
            ),
            ("2.1.10", machine.image.join("2.1.10").join(BINARY)),
            ("2.1.9", machine.home.join("2.1.9").join(BINARY)),
        ]
    );
}

async fn invoke(service: &DemiClaude, operation: &str, input: Vec<u8>) -> (Value, Completion) {
    let cancel = CancellationToken::new();
    let (output, mut records) = Output::channel(cancel.clone());
    let completion = service
        .invoke(InvocationContext {
            request: Invocation {
                operation: operation.into(),
                invocation_id: "invocation".into(),
                context: CommandContext {
                    conversation: "conversation".into(),
                    caller: CommandCaller::agent("caller"),
                    locale: CommandLocale {
                        time_zone: "UTC".into(),
                        languages: vec!["en-US".into()],
                    },
                },
                args: json!({}),
                cwd: "/".into(),
                env: Default::default(),
                edits: None,
                json: None,
            },
            input: Input::from_stream(futures_util::stream::iter([Ok(Bytes::from(input))])),
            output,
            cancellation: cancel,
        })
        .await
        .unwrap();
    let mut stdout = Vec::new();
    while let Some(record) = records.recv().await {
        match record {
            Record::Stdout(bytes) => stdout.extend_from_slice(&bytes),
            other => panic!("unexpected record {other:?}"),
        }
    }
    assert_eq!(stdout.pop(), Some(b'\n'));
    (serde_json::from_slice(&stdout).unwrap(), completion)
}

#[tokio::test]
async fn the_service_answers_one_document_for_each_invocation() {
    let machine = Machine::new();
    let fixture = Fixture::serve(BODY, Duration::ZERO).await;
    let service = DemiClaude::new(machine.installer());
    assert_eq!(service.operations(), ["claude.ensure", "claude.status"]);
    let path = machine.home.join("2.1.278").join(BINARY);

    let (document, completion) = invoke(
        &service,
        "claude.ensure",
        fixture.record("2.1.278", BODY.len(), &sha256(BODY)),
    )
    .await;
    assert_eq!(
        document,
        json!({ "ok": true, "version": "2.1.278", "path": path })
    );
    assert_eq!(
        (completion.exit_code, completion.error.is_none()),
        (0, true)
    );

    let (document, completion) = invoke(&service, "claude.status", Vec::new()).await;
    assert_eq!(
        document,
        json!({
            "ok": true,
            "platform": platform::current().unwrap(),
            "installed": [{ "version": "2.1.278", "path": path }],
        })
    );
    assert_eq!(completion.exit_code, 0);

    let (document, completion) = invoke(
        &service,
        "claude.ensure",
        fixture.record("2.1.279", BODY.len(), &sha256(b"another executable")),
    )
    .await;
    let error = completion.error.unwrap();
    assert_eq!(completion.exit_code, 1);
    assert_eq!(error.code, "verification_failed");
    assert_eq!(
        document,
        json!({ "ok": false, "code": "verification_failed", "message": error.message })
    );

    let (document, completion) = invoke(&service, "claude.ensure", b"{}".to_vec()).await;
    assert_eq!(document["ok"], json!(false));
    assert_eq!(document["code"], json!("invalid_release"));
    assert_eq!(completion.exit_code, 1);

    let (document, _) = invoke(&service, "claude.ensure", vec![b' '; 64 * 1024 + 1]).await;
    assert_eq!(document["code"], json!("invalid_release"));
}
