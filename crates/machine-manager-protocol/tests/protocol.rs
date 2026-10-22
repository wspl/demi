//! The cloud image manifest the packaging writes: the checks that tie its
//! embedded runner and packages to its files.

/// A small manifest in the shape the packaging writes, for an arm64 image
/// embedding one command package.
fn manifest() -> serde_json::Value {
    let runner = serde_json::json!({ "sha256": "a".repeat(64), "size": 38710848 });
    let file = serde_json::json!({ "sha256": "c".repeat(64), "size": 2048 });
    serde_json::json!({
        "formatVersion": 1,
        "os": "linux",
        "architecture": "arm64",
        "rootfs": { "sha256": "d".repeat(64), "size": 733308752, "file": "rootfs.tar.zst" },
        "ubuntu": "26.04",
        "packages": [{ "name": "adduser", "version": "3.153ubuntu1" }],
        "executables": {
            "/opt/demi/bin/demi-runner": runner,
            "/usr/bin/tini": { "sha256": "e".repeat(64), "size": 10 },
            format!("/opt/demi/artifacts/{}/demi-file", "c".repeat(64)): file,
        },
        "releases": [{
            "id": "demi.file",
            "version": "0.1.3",
            "protocolVersion": 1,
            "operations": ["file.read"],
            "targets": { "aarch64-unknown-linux-musl": file },
        }],
        "runner": {
            "release": "f".repeat(64),
            "wire": demi_runner_protocol::wire::VERSION,
            "commandProtocol": 1,
            "targets": { "aarch64-unknown-linux-musl": runner },
        },
    })
}

#[test]
fn an_image_manifest_names_its_embedded_runner_and_packages() {
    use demi_machine_manager_protocol::image::{Architecture, CloudImageManifest, ManifestError};
    let decode =
        |value: &serde_json::Value| CloudImageManifest::decode(&serde_json::to_vec(value).unwrap());
    let decoded = decode(&manifest()).expect("valid manifest");
    assert_eq!(decoded.architecture, Architecture::Arm64);
    assert_eq!(decoded.architecture.target(), "aarch64-unknown-linux-musl");

    let mut changed = manifest();
    changed["executables"]["/opt/demi/bin/demi-runner"]["size"] = 1.into();
    assert!(matches!(decode(&changed), Err(ManifestError::Runner)));
    let mut changed = manifest();
    changed["architecture"] = "amd64".into();
    assert!(matches!(decode(&changed), Err(ManifestError::Runner)));
    let mut changed = manifest();
    changed["executables"]
        .as_object_mut()
        .unwrap()
        .retain(|path, _| !path.starts_with("/opt/demi/artifacts/"));
    assert!(matches!(decode(&changed), Err(ManifestError::Release(id)) if id == "demi.file"));
    for (field, value) in [
        ("formatVersion", serde_json::json!(2)),
        ("os", serde_json::json!("darwin")),
        ("ubuntu", serde_json::json!("")),
        ("extra", serde_json::json!(true)),
    ] {
        let mut changed = manifest();
        changed[field] = value;
        assert!(decode(&changed).is_err(), "{field}");
    }
    for path in [
        "/etc/passwd",
        "/usr/",
        "/usr/../etc/shadow",
        "/opt/x\ny",
        "usr/bin/x",
    ] {
        let mut changed = manifest();
        changed["executables"][path] = serde_json::json!({ "sha256": "e".repeat(64), "size": 10 });
        assert!(decode(&changed).is_err(), "{path:?}");
    }
    let mut changed = manifest();
    changed["rootfs"]["file"] = "rootfs.tar".into();
    assert!(decode(&changed).is_err());
}
