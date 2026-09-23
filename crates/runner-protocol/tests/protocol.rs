//! The runner wire against its golden corpus, the refusals of its decoder,
//! and manifest verification.

use std::path::Path;

use demi_command_service::protocol::PackageDescriptor;
use demi_command_tree::{NativeOperation, Node};
use demi_runner_protocol::manifest::Manifest;
use demi_runner_protocol::wire::{self, Inbound, LogLine, Outbound, Timestamp};
use serde_json::{Value, json};

/// The frames of one direction, recorded with the backend's codec, by name.
fn corpus(direction: &str) -> Vec<(String, Vec<u8>)> {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(direction);
    let mut frames: Vec<_> = std::fs::read_dir(directory)
        .unwrap()
        .map(|entry| {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, std::fs::read(&path).unwrap())
        })
        .collect();
    frames.sort();
    frames
}

fn frame(direction: &str, name: &str) -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(direction)
            .join(format!("{name}.msgpack")),
    )
    .unwrap()
}

fn msgpack(value: &Value) -> Vec<u8> {
    rmp_serde::to_vec_named(value).unwrap()
}

#[test]
fn every_backend_frame_decodes_and_encodes_to_the_same_bytes() {
    let frames = corpus("backend-to-runner");
    assert_eq!(frames.len(), 54);
    for (name, bytes) in frames {
        let message: Inbound =
            wire::decode(&bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(wire::encode(&message).unwrap().into_bytes(), bytes, "{name}");
    }
}

#[test]
fn every_runner_frame_decodes_and_encodes_to_the_same_bytes() {
    let frames = corpus("runner-to-backend");
    assert_eq!(frames.len(), 54);
    for (name, bytes) in frames {
        let message: Outbound =
            wire::decode(&bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(wire::encode(&message).unwrap().into_bytes(), bytes, "{name}");
    }
}

#[test]
fn binary_times_and_contexts_decode_to_their_values() {
    match wire::decode(&frame("backend-to-runner", "spawn_stdin")).unwrap() {
        Inbound::SpawnStdin { spawn_id, bytes } => {
            assert_eq!(spawn_id, "spawn-1");
            assert_eq!(bytes.0, [0, 255, 13, 10]);
        }
        other => panic!("wrong message: {other:?}"),
    }
    match wire::decode(&frame("backend-to-runner", "fs_utimes")).unwrap() {
        Inbound::FsUtimes { atime, mtime, .. } => {
            assert_eq!(atime, Timestamp(-123_456_789));
            assert_eq!(mtime, Timestamp(1_790_146_800_123));
        }
        other => panic!("wrong message: {other:?}"),
    }
    match wire::decode(&frame("backend-to-runner", "job_start")).unwrap() {
        Inbound::JobStart { context, .. } => {
            assert_eq!(context.conversation, "conversation-1");
            assert_eq!(context.caller.node(), Some("node-1"));
            assert_eq!(context.locale.time_zone, "Asia/Shanghai");
            assert_eq!(context.locale.languages, ["zh-CN", "en"]);
        }
        other => panic!("wrong message: {other:?}"),
    }
    match wire::decode(&frame("backend-to-runner", "spawn")).unwrap() {
        Inbound::Spawn { env, .. } => {
            let env = env.unwrap();
            assert_eq!(env["ANTHROPIC_API_KEY"], None);
            assert_eq!(env["PATH"].as_deref(), Some("/usr/bin:/bin"));
        }
        other => panic!("wrong message: {other:?}"),
    }
}

#[test]
fn refuses_disguised_binary_unknown_fields_trailing_data_and_optional_nulls() {
    let invalid = [
        json!({"type": "job_stdin", "jobId": "job", "bytes": [1, 2]}),
        json!({"type": "ping", "extra": true}),
        json!({"type": "fs_stat", "id": "file", "path": "/work", "cwd": null}),
        json!({"type": "hello_error", "code": "unknown", "reason": "no"}),
        json!({"type": "rpc_exit", "callId": "call", "exitCode": 1.5}),
        json!({"type": "net_open", "streamId": "net", "host": "h", "port": 0,
            "input": {"id": "i", "url": "/i"}, "output": {"id": "o", "url": "/o"}}),
    ];
    for message in invalid {
        assert!(wire::decode::<Inbound>(&msgpack(&message)).is_err(), "{message}");
    }
    let valid = json!({"type": "fs_stat", "id": "file", "path": "/work"});
    assert!(wire::decode::<Inbound>(&msgpack(&valid)).is_ok());
    let mut trailing = frame("backend-to-runner", "job_start");
    trailing.push(0);
    assert!(wire::decode::<Inbound>(&trailing).is_err());
}

#[test]
fn nested_values_are_validated_where_the_message_enters() {
    let mut context = json!({
        "conversation": "", "caller": {"kind": "agent", "node": "node"},
        "locale": {"timeZone": "UTC", "languages": ["en"]},
    });
    let job = |context: &Value| {
        json!({"type": "job_start", "jobId": "job", "context": context, "script": "true",
            "cwd": "/", "env": {}})
    };
    assert!(wire::decode::<Inbound>(&msgpack(&job(&context))).is_err());
    context["conversation"] = json!("conversation");
    assert!(wire::decode::<Inbound>(&msgpack(&job(&context))).is_ok());
    context["caller"] = json!({"kind": "user", "node": "node"});
    assert!(wire::decode::<Inbound>(&msgpack(&job(&context))).is_err());
    let mut hashed = job(&json!({
        "conversation": "c", "caller": {"kind": "user"},
        "locale": {"timeZone": "UTC", "languages": ["en"]},
    }));
    hashed["manifestHash"] = json!("not a digest");
    assert!(wire::decode::<Inbound>(&msgpack(&hashed)).is_err());
}

#[test]
fn log_reads_bound_their_limit_and_log_lines_name_their_source() {
    for limit in [0, 1001] {
        let invalid = json!({"type": "log_read", "id": "log", "limit": limit});
        assert!(wire::decode::<Inbound>(&msgpack(&invalid)).is_err());
    }
    let line = LogLine {
        at: Timestamp(1_700_000_000_123),
        source: String::new(),
        conversation_id: None,
        text: "could not list tabs".into(),
    };
    let reply = Outbound::LogLines {
        id: "log".into(),
        lines: vec![line],
        next: 42,
    };
    assert!(wire::encode(&reply).is_err());
}

#[test]
fn replies_name_their_operation_before_its_result() {
    let invalid = [
        json!({"type": "fs_ok", "id": "fs", "result": true, "op": "exists"}),
        json!({"type": "fs_ok", "id": "fs", "op": "unlink", "result": null}),
        json!({"type": "fs_ok", "id": "fs", "op": "mkdir", "result": true}),
        json!({"type": "fs_ok", "id": "fs", "op": "exists"}),
        json!({"type": "fs_ok", "id": "fs", "op": "exists", "result": true, "extra": 1}),
        json!({"type": "git_ok", "id": "git", "op": "show", "result": {}}),
    ];
    for message in invalid {
        assert!(wire::decode::<Outbound>(&msgpack(&message)).is_err(), "{message}");
    }
    let nullable = json!({"type": "spawn_exit", "spawnId": "spawn"});
    assert!(wire::decode::<Outbound>(&msgpack(&nullable)).is_err());
}

#[test]
fn a_working_tree_change_carries_a_pair_git_status_prints() {
    let changes = |status: &str| {
        json!({"type": "git_ok", "id": "git", "op": "changes", "result": {
            "repository": true, "head": null, "truncated": false, "watched": false,
            "files": [{"path": "a", "status": status, "kind": "modified", "added": 0, "removed": 0}],
        }})
    };
    for status in ["M ", " M", "??", "UU", "R "] {
        assert!(wire::decode::<Outbound>(&msgpack(&changes(status))).is_ok(), "{status}");
    }
    for status in ["XY", "M", "  ", "???"] {
        assert!(wire::decode::<Outbound>(&msgpack(&changes(status))).is_err(), "{status}");
    }
}

fn manifest() -> Value {
    serde_json::from_str(include_str!("fixtures/manifest.json")).unwrap()
}

#[test]
fn manifests_verify_their_packages_bindings_and_hash() {
    let parsed = Manifest::parse(manifest()).unwrap();
    assert!(parsed.roots.contains_key("fixture"));
    let mut corrupt = manifest();
    corrupt["roots"]["fixture"]["tree"]["summary"] = "corrupt".into();
    assert!(Manifest::parse(corrupt).is_err());
    let mut duplicated = manifest();
    let descriptor = duplicated["packages"]
        .as_object_mut()
        .unwrap()
        .values_mut()
        .next()
        .unwrap();
    descriptor["operations"] = json!(["file.read", "file.read"]);
    assert!(Manifest::parse(duplicated).is_err());
}

#[test]
fn closed_sets_display_as_the_wire_spells_them() {
    use demi_runner_protocol::wire::{HelloErrorCode, ServiceErrorCode, VolumeName};
    assert_eq!(ServiceErrorCode::UnknownOperation.to_string(), "unknown_operation");
    assert_eq!(HelloErrorCode::AlreadyConnected.to_string(), "already_connected");
    assert_eq!(VolumeName::Home.to_string(), "home");
}

/// A recorded manifest's trees as their declarations: without the descriptor
/// hashes a build pins.
fn declaration(mut tree: Value) -> Node<NativeOperation> {
    fn unpin(node: &mut Value) {
        if let Some(binding) = node.get_mut("binding").and_then(Value::as_object_mut) {
            binding.remove("descriptorHash");
        }
        for child in node
            .get_mut("subcommands")
            .and_then(Value::as_array_mut)
            .into_iter()
            .flatten()
        {
            unpin(child);
        }
    }
    unpin(&mut tree);
    serde_json::from_value(tree).unwrap()
}

#[test]
fn a_built_manifest_pins_its_native_commands_and_hashes_as_the_recorded_one() {
    let recorded = manifest();
    let packages = || {
        recorded["packages"]
            .as_object()
            .unwrap()
            .values()
            .map(|descriptor| PackageDescriptor::parse(descriptor.clone()).unwrap())
            .collect::<Vec<_>>()
    };
    let roots = recorded["roots"]
        .as_object()
        .unwrap()
        .values()
        .map(|root| declaration(root["tree"].clone()));
    let built = Manifest::build(roots, packages()).unwrap();
    assert_eq!(serde_json::to_value(&built).unwrap(), recorded);

    let native = |package: &str, operation: &str| {
        declaration(json!({"name": "native", "summary": "Native", "kind": "native",
            "binding": {"package": package, "operation": operation}}))
    };
    let rpc = || declaration(json!({"name": "rpc", "summary": "Rpc", "kind": "rpc"}));
    let refusal = |roots: Vec<Node<NativeOperation>>, packages: Vec<PackageDescriptor>| {
        Manifest::build(roots, packages).unwrap_err().to_string()
    };
    assert!(refusal(vec![native("demicodes.other", "file.read")], packages()).contains("not configured"));
    assert!(refusal(vec![native("demicodes.fixture", "file.gone")], packages()).contains("no operation"));
    assert!(refusal(vec![rpc(), rpc()], vec![]).contains("duplicate root"));
    let twice = packages().into_iter().chain(packages()).collect();
    assert!(refusal(vec![], twice).contains("duplicate native package"));
}
