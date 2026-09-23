//! The machine-manager socket against the corpus the TypeScript codec
//! recorded (`docs/internal/rust-migration/corpus/record-machines.ts`): every
//! line decodes and encodes to the same bytes, and malformed lines are
//! refused.

use std::{fs, num::NonZeroU64, path::PathBuf};

use demi_machines_protocol::{
    BaseVersion, DeviceId, GenerationId, ImageStateParams, MachineImageState, MachineResponse,
    Operation, RuntimeState, RuntimeStateParams, Volume, decode_request, decode_response,
    encode_line,
};

fn corpus(direction: &str) -> Vec<(String, Vec<u8>)> {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(direction);
    let mut lines: Vec<_> = fs::read_dir(&directory)
        .expect("corpus directory")
        .map(|entry| {
            let path = entry.expect("corpus entry").path();
            let name = path.file_stem().expect("line name").to_string_lossy().into_owned();
            (name, fs::read(&path).expect("corpus line"))
        })
        .collect();
    lines.sort();
    assert!(!lines.is_empty(), "{direction} has no lines");
    lines
}

fn text(line: &[u8]) -> &str {
    std::str::from_utf8(line)
        .expect("a line is UTF-8")
        .strip_suffix('\n')
        .expect("a line ends with a newline")
}

#[test]
fn every_request_decodes_and_encodes_to_the_same_bytes() {
    for (name, line) in corpus("backend-to-manager") {
        let request = decode_request(text(&line)).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(encode_line(&request), line, "{name}");
        // Each request line is named after its operation.
        assert_eq!(request.call.name(), name);
    }
}

#[test]
fn every_response_decodes_and_encodes_to_the_same_bytes() {
    for (name, line) in corpus("manager-to-backend") {
        let response = decode_response(text(&line)).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(encode_line(&response), line, "{name}");
    }
}

/// The result of the `ok` line `name` as `O`'s output, encoded again.
fn result<O: Operation>(name: &str) -> (O::Output, serde_json::Value) {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/manager-to-backend")
        .join(format!("{name}.ndjson"));
    let line = fs::read(path).expect("corpus line");
    let MachineResponse::Ok { result, .. } = decode_response(text(&line)).expect(name) else {
        panic!("{name} is not an ok reply");
    };
    let output: O::Output = serde_json::from_value(result.clone()).expect(name);
    assert_eq!(serde_json::to_value(&output).expect(name), result, "{name}");
    (output, result)
}

#[test]
fn ok_replies_carry_their_operations_results() {
    let (state, _) = result::<ImageStateParams>("ok.image_state");
    assert_eq!(
        state,
        Some(MachineImageState {
            generation: GenerationId::parse("7d2c9b1a-5e4f-4a3b-8c2d-1e0f9a8b7c6d").unwrap(),
            base_version: BaseVersion::parse("b".repeat(64)).unwrap(),
            reset_id: None,
            system_bytes: NonZeroU64::new(1 << 30).unwrap(),
            home_bytes: NonZeroU64::new(2 << 30).unwrap(),
        })
    );
    let (reset, _) = result::<ImageStateParams>("ok.image_state.reset");
    assert_eq!(reset.and_then(|state| state.reset_id).as_deref(), Some("reset-4f1e"));
    assert_eq!(result::<ImageStateParams>("ok.image_state.none").0, None);
    assert_eq!(result::<RuntimeStateParams>("ok.runtime_state").0, RuntimeState::Running);
    assert_eq!(result::<RuntimeStateParams>("ok.runtime_state.stopped").0, RuntimeState::Stopped);
    let (version, _) =
        result::<demi_machines_protocol::CurrentBaseVersionParams>("ok.current_base_version");
    assert_eq!(version.as_str(), "b".repeat(64));
    result::<demi_machines_protocol::ReconcileParams>("ok.reconcile");
}

#[test]
fn requests_ignore_unknown_keys_and_refuse_malformed_values() {
    let device = r#""deviceId":"dev-1""#;
    let boot = r#""boot":{"backendUrl":"http://backend","deviceToken":"opaque"}"#;
    let accepted = [
        format!(r#"{{"id":"1","op":"hibernate","params":{{{device}}},"trace":"x"}}"#),
        format!(r#"{{"id":"1","op":"hibernate","params":{{{device},"force":true}}}}"#),
        r#"{"id":"1","op":"reconcile","params":{"all":true}}"#.to_owned(),
        format!(r#"{{"id":"1","op":"wake","params":{{{device},{boot}}}}}"#),
        // The pattern of a device id or base version is the operation's to check.
        r#"{"id":"1","op":"reset","params":{"deviceId":"../x","operationId":"o","baseVersion":"../b"}}"#
            .to_owned(),
    ];
    for line in &accepted {
        decode_request(line).unwrap_or_else(|error| panic!("{line}: {error}"));
    }
    let refused = [
        "not json".to_owned(),
        r#"{"id":"1","op":"hibernate","params":{"deviceId":"dev-1"}} trailing"#.to_owned(),
        r#"{"id":"1","op":"reboot","params":{}}"#.to_owned(),
        r#"{"id":"1","op":"hibernate"}"#.to_owned(),
        r#"{"op":"reconcile","params":{}}"#.to_owned(),
        r#"{"id":"","op":"reconcile","params":{}}"#.to_owned(),
        r#"{"id":"1","op":"hibernate","params":{"deviceId":""}}"#.to_owned(),
        r#"{"id":"1","op":"hibernate","params":{"deviceId":7}}"#.to_owned(),
        format!(r#"{{"id":"1","op":"wake","params":{{{device}}}}}"#),
        format!(
            r#"{{"id":"1","op":"wake","params":{{{device},"boot":{{"backendUrl":"http://backend","deviceToken":"opaque","command":"x"}}}}}}"#
        ),
        format!(
            r#"{{"id":"1","op":"wake","params":{{{device},"boot":{{"backendUrl":"ftp://backend","deviceToken":"opaque"}}}}}}"#
        ),
        format!(
            r#"{{"id":"1","op":"wake","params":{{{device},"boot":{{"backendUrl":"http://backend","deviceToken":"two words"}}}}}}"#
        ),
        format!(r#"{{"id":"1","op":"grow_volume","params":{{{device},"volume":"home","bytes":0}}}}"#),
        format!(r#"{{"id":"1","op":"grow_volume","params":{{{device},"volume":"home","bytes":-1}}}}"#),
        format!(r#"{{"id":"1","op":"grow_volume","params":{{{device},"volume":"home","bytes":1.5}}}}"#),
        format!(r#"{{"id":"1","op":"grow_volume","params":{{{device},"volume":"swap","bytes":1}}}}"#),
        format!(r#"{{"id":"1","op":"reset","params":{{{device},"operationId":"","baseVersion":"b"}}}}"#),
        format!(r#"{{"id":"1","op":"reset","params":{{{device},"operationId":"o"}}}}"#),
    ];
    for line in &refused {
        assert!(decode_request(line).is_err(), "{line} decoded");
    }
}

#[test]
fn responses_refuse_unknown_types_and_missing_names() {
    for line in [
        r#"{"type":"done","id":"1"}"#,
        r#"{"type":"ok","result":null}"#,
        r#"{"type":"error","id":"1"}"#,
        r#"{"type":"death","deviceId":""}"#,
    ] {
        assert!(decode_response(line).is_err(), "{line} decoded");
    }
}

#[test]
fn an_image_state_requires_every_key_and_valid_names() {
    let valid = r#"{"generation":"g-1","baseVersion":"b_1","resetId":null,"systemBytes":1,"homeBytes":2}"#;
    let state: MachineImageState = serde_json::from_str(valid).expect("valid state");
    assert_eq!(state.bytes(Volume::Home).get(), 2);
    for invalid in [
        r#"{"generation":"g-1","baseVersion":"b_1","systemBytes":1,"homeBytes":2}"#,
        r#"{"generation":"g/1","baseVersion":"b_1","resetId":null,"systemBytes":1,"homeBytes":2}"#,
        r#"{"generation":"g-1","baseVersion":"","resetId":null,"systemBytes":1,"homeBytes":2}"#,
        r#"{"generation":"g-1","baseVersion":"b_1","resetId":null,"systemBytes":0,"homeBytes":2}"#,
        r#"{"generation":"g-1","baseVersion":"b_1","resetId":null,"systemBytes":1,"homeBytes":2,"x":1}"#,
    ] {
        assert!(
            serde_json::from_str::<MachineImageState>(invalid).is_err(),
            "{invalid} decoded"
        );
    }
}

#[test]
fn image_names_are_single_path_components() {
    for name in ["dev-1", "A_b-9", "0f6c3d4e-8a9b-4c1d-9e2f-3a4b5c6d7e8f"] {
        DeviceId::parse(name).unwrap_or_else(|error| panic!("{error}"));
    }
    for name in ["", ".", "..", "a/b", "a b", "dev.1", "é"] {
        assert!(DeviceId::parse(name).is_err(), "{name:?} parsed");
    }
}

#[test]
fn a_boot_credential_never_appears_in_debugging_output() {
    let line = fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/backend-to-manager/wake.ndjson"),
    )
    .expect("wake line");
    let request = decode_request(text(&line)).expect("wake");
    let debug = format!("{request:?}");
    assert!(debug.contains("backend.example.com"), "{debug}");
    assert!(!debug.contains("dt_6b1f0c2e9a7d4c3b"), "{debug}");
}

/// A small manifest in the shape the packaging writes, for an arm64 image
/// embedding one command package.
fn manifest() -> serde_json::Value {
    let runner = serde_json::json!({ "sha256": "a".repeat(64), "size": 38710848 });
    let builtin = serde_json::json!({ "sha256": "c".repeat(64), "size": 2048 });
    serde_json::json!({
        "formatVersion": 1,
        "os": "linux",
        "architecture": "arm64",
        "rootfs": { "sha256": "d".repeat(64), "size": 733308752, "file": "rootfs.tar.zst" },
        "ubuntu": "26.04",
        "packages": [{ "name": "adduser", "version": "3.153ubuntu1" }],
        "executables": {
            "/usr/bin/demi-runner": runner,
            "/usr/bin/tini": { "sha256": "e".repeat(64), "size": 10 },
            format!("/opt/demi/artifacts/{}/demi-commands", "c".repeat(64)): builtin,
        },
        "releases": [{
            "id": "demi.builtin",
            "version": "0.1.3",
            "protocolVersion": 1,
            "operations": ["file.read"],
            "targets": { "aarch64-unknown-linux-musl": builtin },
        }],
        "runner": {
            "release": "f".repeat(64),
            "wire": 18,
            "commandProtocol": 1,
            "targets": { "aarch64-unknown-linux-musl": runner },
        },
        "tools": [{ "name": "uv", "version": "0.12.13", "sha256": "9".repeat(64) }],
    })
}

#[test]
fn an_image_manifest_names_its_embedded_runner_and_packages() {
    use demi_machines_protocol::image::{Architecture, CloudImageManifest, ManifestError};
    let decode = |value: &serde_json::Value| CloudImageManifest::decode(&serde_json::to_vec(value).unwrap());
    let decoded = decode(&manifest()).expect("valid manifest");
    assert_eq!(decoded.architecture, Architecture::Arm64);
    assert_eq!(decoded.architecture.target(), "aarch64-unknown-linux-musl");

    let mut changed = manifest();
    changed["executables"]["/usr/bin/demi-runner"]["size"] = 1.into();
    assert!(matches!(decode(&changed), Err(ManifestError::Runner)));
    let mut changed = manifest();
    changed["architecture"] = "amd64".into();
    assert!(matches!(decode(&changed), Err(ManifestError::Runner)));
    let mut changed = manifest();
    changed["executables"].as_object_mut().unwrap().retain(|path, _| !path.starts_with("/opt/"));
    assert!(matches!(decode(&changed), Err(ManifestError::Release(id)) if id == "demi.builtin"));
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
    for path in ["/etc/passwd", "/usr/", "/usr/../etc/shadow", "/opt/x\ny", "usr/bin/x"] {
        let mut changed = manifest();
        changed["executables"][path] = serde_json::json!({ "sha256": "e".repeat(64), "size": 10 });
        assert!(decode(&changed).is_err(), "{path:?}");
    }
    let mut changed = manifest();
    changed["rootfs"]["file"] = "rootfs.tar".into();
    assert!(decode(&changed).is_err());
}
