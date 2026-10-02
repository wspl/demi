//! The package's rules (`claude-code.md` § The package): the executable it
//! asks the runner for, from which release record, and why it refuses. The
//! runner's artifacts stream is answered by a test source, as the runner
//! would answer it.

use std::sync::{Arc, Mutex};

use demi_command_protocol::{ArtifactAsk, ArtifactForm, ArtifactReply, InstalledArtifact};
use demi_command_sdk::testing::artifacts_from;
use serde_json::json;
use sha2::{Digest, Sha256};

use demi_claude_code::{
    install::{self, EnsureError},
    platform::{self, Loaders, platform_key},
};

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// A release record of `version` with an entry for `platform`.
fn record(version: &str, platform: &str) -> Vec<u8> {
    json!({
        "version": version,
        "platforms": { platform: { "url": "https://downloads.claude.ai/claude", "size": 6, "sha256": sha256(b"claude") } },
    })
    .to_string()
    .into_bytes()
}

/// `ensure` asks the runner, for its invocation, to install this machine's
/// executable of the release as the file `Claude Code` at the release's
/// version, and answers the path the runner gives.
#[tokio::test]
async fn ensure_asks_the_runner_for_this_machine_s_executable() {
    let asked = Arc::new(Mutex::new(Vec::new()));
    let seen = asked.clone();
    let artifacts = artifacts_from(move |ask| {
        seen.lock().unwrap().push(ask);
        Ok(ArtifactReply::Path("/cache/claude".into()))
    });
    let platform = platform::current().unwrap();
    let release = install::release(&record("2.1.278", platform)).unwrap();
    let installed = install::ensure(&artifacts, "invocation", &release)
        .await
        .unwrap();
    assert_eq!(
        (installed.version.as_str(), installed.path.to_str().unwrap()),
        ("2.1.278", "/cache/claude")
    );
    let asked = asked.lock().unwrap();
    let [ArtifactAsk::Install(install)] = asked.as_slice() else {
        panic!("one install asked: {asked:?}");
    };
    assert_eq!(
        (
            install.invocation.as_str(),
            install.name.as_str(),
            install.version.as_str(),
            install.sha256.as_str(),
            install.size,
            &install.form,
        ),
        (
            "invocation",
            install::NAME,
            "2.1.278",
            sha256(b"claude").as_str(),
            6,
            &ArtifactForm::File,
        )
    );
}

/// A release without this machine's platform is unsupported and asks the
/// runner nothing; an install the runner could not make fails with its
/// reason.
#[tokio::test]
async fn an_unsupported_platform_asks_nothing_and_a_runner_failure_is_an_install_failure() {
    let asked = Arc::new(Mutex::new(0));
    let counted = asked.clone();
    let artifacts = artifacts_from(move |_| {
        *counted.lock().unwrap() += 1;
        Err("the download failed".into())
    });
    let elsewhere = install::release(&record("2.1.278", "other-platform")).unwrap();
    let refused = install::ensure(&artifacts, "invocation", &elsewhere).await;
    assert!(
        matches!(refused, Err(EnsureError::UnsupportedPlatform(_))),
        "{refused:?}"
    );
    assert_eq!(*asked.lock().unwrap(), 0);
    let release = install::release(&record("2.1.278", platform::current().unwrap())).unwrap();
    let failed = install::ensure(&artifacts, "invocation", &release).await;
    let Err(error @ EnsureError::InstallFailed(_)) = failed else {
        panic!("{failed:?}");
    };
    assert!(error.to_string().contains("the download failed"), "{error}");
}

/// `status` answers the versions the runner has of the line, in its order.
#[tokio::test]
async fn status_answers_the_runner_s_versions() {
    let artifacts = artifacts_from(|ask| {
        let ArtifactAsk::Installed(question) = ask else {
            return Err("only a question".into());
        };
        assert_eq!(question.name, install::NAME);
        let installed = |version: &str| InstalledArtifact {
            version: version.into(),
            sha256: sha256(version.as_bytes()),
            path: format!("/cache/{version}"),
        };
        Ok(ArtifactReply::Installed(vec![
            installed("2.1.278"),
            installed("2.1.10"),
        ]))
    });
    let status = install::status(&artifacts).await.unwrap();
    assert_eq!(status.platform, platform::current().unwrap());
    let versions: Vec<&str> = status
        .installed
        .iter()
        .map(|installed| installed.version.as_str())
        .collect();
    assert_eq!(versions, ["2.1.278", "2.1.10"]);
}

#[test]
fn malformed_records_are_invalid() {
    let digest = sha256(b"claude");
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
        let error = install::release(record.to_string().as_bytes()).unwrap_err();
        assert_eq!(
            error.code().map(|code| code.to_string()).as_deref(),
            Some("invalid_release"),
            "{record}"
        );
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
        let error = install::release(record.to_string().as_bytes()).unwrap_err();
        assert_eq!(
            error.code().map(|code| code.to_string()).as_deref(),
            Some("invalid_release"),
            "{version:?}"
        );
    }
    assert!(install::release(b"{").is_err());
    for version in ["2.1.278", "0.0.0", "10.20.30-beta.1", "1.0.0-rc-1"] {
        let record = json!({ "version": version, "platforms": { "linux-x64": artifact } });
        assert!(
            install::release(record.to_string().as_bytes()).is_ok(),
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
