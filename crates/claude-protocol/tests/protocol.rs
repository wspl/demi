//! The `demi.claude` records and documents.

use demi_claude_protocol::{ErrorCode, Failure, Installed, Release, Reply, Status, is_version};
use serde_json::json;

fn record(version: &str, url: &str, size: u64, sha256: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({
        "version": version,
        "platforms": {"darwin-arm64": {"url": url, "size": size, "sha256": sha256}},
    }))
    .unwrap()
}

#[test]
fn a_release_record_is_checked_in_every_entry() {
    let digest = "a".repeat(64);
    let release = Release::parse(&record("2.1.3-beta.1", "https://example.test/claude", 3, &digest)).unwrap();
    assert_eq!(release.platforms["darwin-arm64"].size, 3);
    for invalid in [
        record("2.1", "https://example.test/claude", 3, &digest),
        record("2.1.3", "not a url", 3, &digest),
        record("2.1.3", "https://example.test/claude", 0, &digest),
        record("2.1.3", "https://example.test/claude", 3, &"A".repeat(64)),
    ] {
        assert!(Release::parse(&invalid).is_err());
    }
    assert!(Release::parse(br#"{"version": "2.1.3", "platforms": {}, "extra": 1}"#).is_err());
}

#[test]
fn versions_are_three_numbers_and_an_optional_prerelease() {
    for valid in ["1.0.0", "10.20.30-rc.1", "0.0.1-alpha-2"] {
        assert!(is_version(valid), "{valid}");
    }
    for invalid in ["1.0", "1.0.0-", "1.0.0+build", "v1.0.0", "1..0", "../1.0.0"] {
        assert!(!is_version(invalid), "{invalid}");
    }
}

#[test]
fn replies_carry_ok_beside_the_answer() {
    let status = Reply::Done(Status {
        platform: "darwin-arm64".into(),
        installed: vec![Installed { version: "2.1.3".into(), path: "/opt/claude".into() }],
    });
    let printed = serde_json::to_value(&status).unwrap();
    assert_eq!(
        printed,
        json!({"ok": true, "platform": "darwin-arm64",
            "installed": [{"version": "2.1.3", "path": "/opt/claude"}]}),
    );
    assert_eq!(serde_json::from_value::<Reply<Status>>(printed).unwrap(), status);
    let failed: Reply<Status> = Reply::Failed(Failure {
        code: ErrorCode::VerificationFailed,
        message: "digest differs".into(),
    });
    let printed = serde_json::to_value(&failed).unwrap();
    assert_eq!(printed, json!({"ok": false, "code": "verification_failed", "message": "digest differs"}));
    assert_eq!(serde_json::from_value::<Reply<Status>>(printed).unwrap(), failed);
    assert!(serde_json::from_value::<Reply<Status>>(json!({"platform": "x", "installed": []})).is_err());
}
