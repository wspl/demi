//! The package's rules (`crates/command-package-claude-code/src/lib.rs`): which
//! release records it refuses.

use serde_json::json;
use sha2::{Digest, Sha256};

use demi_claude_code::install;

fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
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
