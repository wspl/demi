//! The runner wire against its golden corpus, the refusals of its decoder,
//! kept output records, manifest verification and the runner release record.

use std::path::Path;
use std::time::{Duration, Instant};

use demi_command_declarations::{NativeOperation, Node};
use demi_command_protocol::PackageDescriptor;
use demi_runner_protocol::files::{
    Effect, FileWatchMessage, MAX_OUTSIDE_SUBSCRIPTIONS, MAX_PAGE_SUBSCRIPTIONS, PageSubscription,
    PageSubscriptions, RefusedReason, Subject, SubscriptionKind,
};
use demi_runner_protocol::installation::installation_name;
use demi_runner_protocol::manifest::Manifest;
use demi_runner_protocol::wire::{
    self, Inbound, KeptRecord, LogLine, Outbound, OutputStream, Timestamp, WireBytes,
};
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
    assert_eq!(frames.len(), 63);
    for (name, bytes) in frames {
        let message: Inbound =
            wire::decode(&bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            wire::encode(&message).unwrap().into_bytes(),
            bytes,
            "{name}"
        );
    }
}

#[test]
fn every_runner_frame_decodes_and_encodes_to_the_same_bytes() {
    let frames = corpus("runner-to-backend");
    assert_eq!(frames.len(), 61);
    for (name, bytes) in frames {
        let message: Outbound =
            wire::decode(&bytes).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            wire::encode(&message).unwrap().into_bytes(),
            bytes,
            "{name}"
        );
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
            assert_eq!(context.caller.agent_number(), Some(1));
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
        json!({"type": "log_read", "id": "log", "limit": 0}),
    ];
    for message in invalid {
        assert!(
            wire::decode::<Inbound>(&msgpack(&message)).is_err(),
            "{message}"
        );
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
        "conversation": "", "caller": {"kind": "agent", "number": 1},
        "locale": {"timeZone": "UTC", "languages": ["en"]}, "colorScheme": "light",
    });
    let job = |context: &Value| {
        json!({"type": "job_start", "jobId": "job", "context": context, "script": "true",
            "cwd": "/", "env": {}})
    };
    assert!(wire::decode::<Inbound>(&msgpack(&job(&context))).is_err());
    context["conversation"] = json!("conversation");
    assert!(wire::decode::<Inbound>(&msgpack(&job(&context))).is_ok());
    context["caller"] = json!({"kind": "user", "number": 1});
    assert!(wire::decode::<Inbound>(&msgpack(&job(&context))).is_err());
    let mut hashed = job(&json!({
        "conversation": "c", "caller": {"kind": "user"},
        "locale": {"timeZone": "UTC", "languages": ["en"]}, "colorScheme": "light",
    }));
    hashed["manifestHash"] = json!("not a digest");
    assert!(wire::decode::<Inbound>(&msgpack(&hashed)).is_err());
}

/// The runner names a job's directory after its conversation, and a release
/// removes that directory (`crates/runner-jobs/src/tasks.rs`): every place the
/// wire carries a conversation's name refuses one that could name another
/// path.
#[test]
fn a_conversation_name_is_letters_digits_dashes_and_underscores() {
    let long = "a".repeat(65);
    let names = [
        ("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a01", true),
        ("provider_entry-1", true),
        (&long[1..], true),
        ("..", false),
        ("../jobs", false),
        ("a/b", false),
        ("", false),
        (long.as_str(), false),
    ];
    for (name, valid) in names {
        let context = json!({
            "conversation": name, "caller": {"kind": "user"},
            "locale": {"timeZone": "UTC", "languages": ["en"]}, "colorScheme": "light",
        });
        let job = json!({"type": "job_start", "jobId": "job", "context": context, "script": "true",
            "cwd": "/", "env": {}});
        let release =
            json!({"type": "conversation_release", "id": "release", "conversationId": name});
        for message in [job, release] {
            assert_eq!(
                wire::decode::<Inbound>(&msgpack(&message)).is_ok(),
                valid,
                "{message}"
            );
        }
    }
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
fn a_numbers_request_asks_for_one_to_sixteen_numbers_of_a_named_sequence() {
    let request = |count: u64, sequence: &str| {
        json!({"type": "numbers_reserve", "id": "numbers", "conversationId": "c",
            "sequence": sequence, "count": count})
    };
    for (count, sequence, valid) in [
        (1, "tab", true),
        (16, "tab", true),
        (0, "tab", false),
        (17, "tab", false),
        (1, "command", false),
    ] {
        let decoded = wire::decode::<Outbound>(&msgpack(&request(count, sequence)));
        assert_eq!(decoded.is_ok(), valid, "{count} {sequence}");
    }
    let answer = |first: u64| json!({"type": "numbers_reserved", "id": "numbers", "first": first});
    assert!(wire::decode::<Inbound>(&msgpack(&answer(1))).is_ok());
    assert!(wire::decode::<Inbound>(&msgpack(&answer(0))).is_err());
}

/// A Host's number, as every fs and working-tree reply carries one.
fn read_at() -> Value {
    json!({ "epoch": "5f1a2b3c4d5e6f70", "number": 42 })
}

#[test]
fn replies_name_their_operation_before_its_result() {
    let invalid = [
        json!({"type": "fs_ok", "id": "fs", "readAt": read_at(), "result": true, "op": "exists"}),
        json!({"type": "fs_ok", "id": "fs", "readAt": read_at(), "op": "unlink", "result": null}),
        json!({"type": "fs_ok", "id": "fs", "readAt": read_at(), "op": "mkdir", "result": true}),
        json!({"type": "fs_ok", "id": "fs", "readAt": read_at(), "op": "exists"}),
        json!({"type": "fs_ok", "id": "fs", "readAt": read_at(), "op": "exists", "result": true, "extra": 1}),
        json!({"type": "git_ok", "id": "git", "readAt": read_at(), "op": "show", "result": {}}),
        // Every reply says the Host's number before it read.
        json!({"type": "fs_ok", "id": "fs", "op": "exists", "result": true}),
    ];
    for message in invalid {
        assert!(
            wire::decode::<Outbound>(&msgpack(&message)).is_err(),
            "{message}"
        );
    }
    let nullable = json!({"type": "spawn_exit", "spawnId": "spawn"});
    assert!(wire::decode::<Outbound>(&msgpack(&nullable)).is_err());
}

#[test]
fn a_working_tree_change_carries_a_pair_git_status_prints() {
    let changes = |status: &str| {
        json!({"type": "git_ok", "id": "git", "readAt": read_at(), "op": "changes", "result": {
            "repository": true, "gitDir": "/work/.git", "head": null, "truncated": false, "watched": false,
            "files": [{"path": "a", "status": status, "kind": "modified", "added": 0, "removed": 0}],
        }})
    };
    for status in ["M ", " M", "??", "UU", "R "] {
        assert!(
            wire::decode::<Outbound>(&msgpack(&changes(status))).is_ok(),
            "{status}"
        );
    }
    for status in ["XY", "M", "  ", "???"] {
        assert!(
            wire::decode::<Outbound>(&msgpack(&changes(status))).is_err(),
            "{status}"
        );
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
    // A received tree follows the declaration rules, whatever built it.
    let mut contradictory = manifest();
    contradictory["roots"]["fixture"]["tree"]["subcommands"][0]["positionals"] =
        json!(["body", "path"]);
    let error = Manifest::parse(contradictory).unwrap_err().to_string();
    assert!(error.contains("multiple input sources for body"), "{error}");
}

#[test]
fn a_hello_names_its_platform_as_a_device_stores_it_and_refuses_any_other_name() {
    use demi_runner_protocol::wire::RunnerPlatform;
    let mut hello: Value = rmp_serde::from_slice(&frame("runner-to-backend", "hello")).unwrap();
    for (name, platform) in [
        ("darwin", RunnerPlatform::Darwin),
        ("win32", RunnerPlatform::Win32),
        ("linux", RunnerPlatform::Linux),
    ] {
        hello["runner"]["platform"] = json!(name);
        match wire::decode(&msgpack(&hello)).unwrap() {
            Outbound::Hello { runner, .. } => assert_eq!(runner.platform, platform),
            other => panic!("{other:?}"),
        }
        // A device stores its platform by this name.
        assert_eq!(platform.to_string(), name);
        assert_eq!(name.parse::<RunnerPlatform>().unwrap(), platform);
    }
    hello["runner"]["platform"] = json!("macos");
    assert!(wire::decode::<Outbound>(&msgpack(&hello)).is_err());
}

#[test]
fn a_hello_names_the_hosts_operating_system_and_is_refused_without_one() {
    let mut hello: Value = rmp_serde::from_slice(&frame("runner-to-backend", "hello")).unwrap();
    match wire::decode(&msgpack(&hello)).unwrap() {
        Outbound::Hello { runner, .. } => {
            assert_eq!(runner.os.name, "macOS 26.5");
            assert_eq!(runner.os.arch, "aarch64");
        }
        other => panic!("{other:?}"),
    }
    hello["runner"]["os"]["name"] = json!("");
    assert!(wire::decode::<Outbound>(&msgpack(&hello)).is_err());
    hello["runner"].as_object_mut().unwrap().remove("os");
    assert!(wire::decode::<Outbound>(&msgpack(&hello)).is_err());
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
        declaration(
            json!({"name": "native", "summary": "Native", "kind": "native",
            "binding": {"package": package, "operation": operation}}),
        )
    };
    let rpc = || declaration(json!({"name": "rpc", "summary": "Rpc", "kind": "rpc"}));
    let refusal = |roots: Vec<Node<NativeOperation>>, packages: Vec<PackageDescriptor>| {
        Manifest::build(roots, packages).unwrap_err().to_string()
    };
    assert!(
        refusal(vec![native("demicodes.other", "file.read")], packages())
            .contains("not configured")
    );
    assert!(
        refusal(vec![native("demicodes.fixture", "file.gone")], packages())
            .contains("no operation")
    );
    assert!(refusal(vec![rpc(), rpc()], vec![]).contains("duplicate root"));
    let twice = packages().into_iter().chain(packages()).collect();
    assert!(refusal(vec![], twice).contains("duplicate command package"));
    let contradictory = declaration(json!({"name": "note", "summary": "Note", "kind": "rpc",
        "input": {"type": "object", "properties": {"text": {"type": "string"}}},
        "positionals": ["text"], "restField": "text"}));
    assert!(refusal(vec![contradictory], vec![]).contains("multiple input sources for text"));

    // The hash covers every declaration and every descriptor, since a runner
    // keeps an installed manifest until the hash changes.
    let hash = |roots: Vec<Node<NativeOperation>>, packages: Vec<PackageDescriptor>| {
        Manifest::build(roots, packages).unwrap().hash
    };
    let hinted = |hint: &str| {
        declaration(json!({"name": "rpc", "summary": "Rpc", "kind": "rpc", "runningHint": hint}))
    };
    assert_ne!(
        hash(vec![hinted("working")], vec![]),
        hash(vec![hinted("still working")], vec![])
    );
    let mut released = packages();
    released[0].version = "next".into();
    assert_ne!(hash(vec![rpc()], released), hash(vec![rpc()], packages()));
}

/// A command's output as a runner kept it and the backend stores it for 30
/// days: records written by one version decode in the next.
#[test]
fn a_kept_output_decodes_from_its_recorded_bytes() {
    let bytes = std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/kept/output.msgpack"),
    )
    .unwrap();
    let records = wire::decode_records(&bytes).unwrap();
    assert_eq!(
        records,
        [
            KeptRecord::Output(OutputStream::Stdout, WireBytes(b"first\n".to_vec())),
            KeptRecord::Output(OutputStream::Stderr, WireBytes(vec![0, 255, 10])),
            KeptRecord::LeftOut(734_003_200),
            KeptRecord::Output(OutputStream::Stdout, WireBytes(b"last\n".to_vec())),
        ]
    );
    let encoded: Vec<u8> = records
        .iter()
        .flat_map(|record| wire::encode_record(record).unwrap())
        .collect();
    assert_eq!(encoded, bytes);
}

/// A release record as `xtask native package` writes it, compacted,
/// for the wire this build speaks.
fn record() -> String {
    format!(
        r#"{{"release":"317dd84e2ce0846a1bea4bc5959959c04af7ba8e4de32b3752fdd6b409f7b1a5","wire":{},"commandProtocol":1,"targets":{{"aarch64-apple-darwin":{{"sha256":"dbf18cb3af50a3348a834ea9cee7981f7354f84aa89764f4d68812c81ebe045b","size":38772096}},"aarch64-unknown-linux-musl":{{"sha256":"5d4219232ad6a95b2a0e72097011e91ae3773e8523e8032788904fa0e9164197","size":38710848}}}}}}"#,
        wire::VERSION
    )
}

#[test]
fn a_release_record_is_checked_in_every_field() {
    let record = record();
    demi_runner_protocol::release::RunnerRelease::decode(record.as_bytes())
        .expect("the record as written is valid");
    let wire = format!(r#""wire":{}"#, wire::VERSION);
    for (from, to) in [
        (
            r#""release":"317dd84e"#.to_owned(),
            r#""release":"317DD84E"#.to_owned(),
        ),
        (wire.clone(), format!(r#""wire":{}"#, wire::VERSION - 1)),
        (
            r#""commandProtocol":1"#.to_owned(),
            r#""commandProtocol":2"#.to_owned(),
        ),
        (
            "aarch64-apple-darwin".to_owned(),
            "aarch64-apple-ios".to_owned(),
        ),
        (r#""size":38772096"#.to_owned(), r#""size":0"#.to_owned()),
        (wire.clone(), format!(r#"{wire},"channel":"beta""#)),
    ] {
        let changed = record.replacen(&from, &to, 1);
        assert!(
            demi_runner_protocol::release::RunnerRelease::decode(changed.as_bytes()).is_err(),
            "{changed}"
        );
    }
}

/// A kept output at its bound, with the largest count of bytes left out,
/// is within what a `job_read` may carry.
#[test]
fn the_record_between_the_parts_fits_the_read_bound() {
    let gap = wire::encode_record(&KeptRecord::LeftOut(u64::MAX)).unwrap();
    assert!(wire::JOB_KEPT_BYTES + gap.len() <= wire::JOB_KEPT_READ_BYTES);
}

#[test]
fn records_round_trip_and_a_second_gap_is_refused() {
    let records = [
        KeptRecord::Output(OutputStream::Stdout, WireBytes(b"a".to_vec())),
        KeptRecord::LeftOut(7),
        KeptRecord::Output(OutputStream::Stderr, WireBytes(b"b".to_vec())),
    ];
    let bytes: Vec<u8> = records
        .iter()
        .flat_map(|record| wire::encode_record(record).unwrap())
        .collect();
    assert_eq!(wire::decode_records(&bytes).unwrap(), records);
    let twice = [
        bytes.clone(),
        wire::encode_record(&KeptRecord::LeftOut(1)).unwrap(),
    ]
    .concat();
    assert!(wire::decode_records(&twice).is_err());
    assert!(wire::decode_records(&bytes[..bytes.len() - 1]).is_err());
}

/// A backend's URL names a host over HTTP or a WebSocket, and never carries
/// credentials or a fragment (`crates/runner-protocol/src/wire/mod.rs`): the
/// installation, the claim and the boot record are refused otherwise.
#[test]
fn a_backend_url_names_a_host_without_credentials_or_a_fragment() {
    use demi_runner_protocol::values::BackendUrl;

    for accepted in [
        "https://demi.example.com",
        "http://10.0.0.5:3271",
        "wss://demi.example.com/runner",
    ] {
        assert!(accepted.parse::<BackendUrl>().is_ok(), "{accepted}");
    }
    for refused in [
        "https://user:pass@demi.example.com",
        "https://user@demi.example.com",
        "https://demi.example.com/#runner",
        "ftp://demi.example.com",
        "file:///tmp/backend",
        "not a url",
    ] {
        assert!(refused.parse::<BackendUrl>().is_err(), "{refused}");
    }
}


#[test]
fn a_files_version_is_its_size_and_time_in_hexadecimal_as_a_weak_etag() {
    assert_eq!(
        wire::file_version(300_000, 1_790_000_000_123),
        "W/\"493e0-1a0c4506c7b\""
    );
    assert_eq!(wire::file_version(0, -16), "W/\"0--10\"");
}
#[test]
fn a_delete_keeps_the_roots_and_every_directory_that_holds_a_kept_one() {
    use demi_runner_protocol::files::protected_path;
    let kept = ["/home/ana", "/home/ana/work"];
    for path in [
        "/",
        "/home",
        "/home/ana",
        "/HOME/Ana/",
        "/home/ana/work/..",
        "/home/ana/work",
        "C:\\",
    ] {
        assert!(protected_path(path, kept), "{path}");
    }
    for path in ["/home/ana/work/notes.md", "/home/ana/other", "/home/anabel"] {
        assert!(!protected_path(path, kept), "{path}");
    }
}


/// One subscription a page asks for.
fn asked(key: &str, kind: SubscriptionKind, path: &str) -> PageSubscription {
    PageSubscription {
        key: key.into(),
        kind,
        path: path.into(),
        suffix: None,
    }
}

fn subject(kind: SubscriptionKind, path: &str) -> Subject {
    Subject {
        kind,
        path: path.into(),
        suffix: None,
    }
}

/// The subjects `effects` hold and release on the Host, and the messages
/// for the page, in order.
fn split(effects: Vec<Effect>) -> (Vec<Subject>, Vec<Subject>, Vec<FileWatchMessage>) {
    let (mut held, mut released, mut page) = (Vec::new(), Vec::new(), Vec::new());
    for effect in effects {
        match effect {
            Effect::Hold(subject) => held.push(subject),
            Effect::Release(subject) => released.push(subject),
            Effect::Page(message) => page.push(message),
            Effect::Renew(_) => {}
        }
    }
    (held, released, page)
}

/// Two keys of one subject share one Host subscription, the second hears
/// `live` at once once it runs, and the subject is let go with its last
/// key; a subscription inside the conversation's working tree also holds
/// the tree's own, which the page hears of only under a key of its own.
#[test]
fn a_pages_keys_share_one_host_subscription_and_hold_the_working_tree_while_inside_it() {
    use SubscriptionKind::*;
    let now = Instant::now();
    let mut page = PageSubscriptions::new(Some("/work".into()));
    let tree = subject(Worktree, "/work");
    let file = subject(File, "/work/a.rs");
    let (held, _, heard) = split(page.subscribe(vec![asked("k1", File, "/work/a.rs")], now));
    assert_eq!(held, [tree.clone(), file.clone()]);
    assert!(heard.is_empty());
    assert!(page.live(&tree, 3, "e").is_empty(), "nothing the page holds is the tree's");
    let live = page.live(&file, 5, "e");
    assert_eq!(live, [Effect::Page(FileWatchMessage::Live { key: "k1".into(), since: 5, epoch: "e".into() })]);
    let (held, _, heard) = split(page.subscribe(vec![asked("k2", File, "/work/a.rs")], now));
    assert!(held.is_empty(), "one subscription of the subject");
    assert_eq!(heard, [FileWatchMessage::Live { key: "k2".into(), since: 5, epoch: "e".into() }]);
    assert_eq!(
        page.changed(&[(file.clone(), 9), (tree.clone(), 8)]),
        [Effect::Page(FileWatchMessage::Changed { changes: vec![("k1".into(), 9), ("k2".into(), 9)] })]
    );
    let (_, released, _) = split(page.unsubscribe(&["k1".into()]));
    assert!(released.is_empty());
    let (_, released, _) = split(page.unsubscribe(&["k2".into()]));
    assert_eq!(released, [tree, file]);
}

/// A page holds at most 4,096 subscriptions, 64 of them outside the
/// working tree; one past either is refused, and one that fails is over
/// alone.
#[test]
fn past_its_limits_a_page_is_refused_and_one_failure_ends_only_its_own() {
    use SubscriptionKind::*;
    let now = Instant::now();
    let mut page = PageSubscriptions::new(Some("/work".into()));
    let outside: Vec<_> = (0..=MAX_OUTSIDE_SUBSCRIPTIONS)
        .map(|index| asked(&format!("o{index}"), File, &format!("/tmp/{index}.log")))
        .collect();
    let (_, _, heard) = split(page.subscribe(outside, now));
    let refused = FileWatchMessage::Refused { key: format!("o{MAX_OUTSIDE_SUBSCRIPTIONS}"), reason: RefusedReason::Limit };
    assert_eq!(heard, [refused]);
    let inside: Vec<_> = (0..=MAX_PAGE_SUBSCRIPTIONS - MAX_OUTSIDE_SUBSCRIPTIONS)
        .map(|index| asked(&format!("i{index}"), File, &format!("/work/{index}.rs")))
        .collect();
    let (_, _, heard) = split(page.subscribe(inside, now));
    let last = MAX_PAGE_SUBSCRIPTIONS - MAX_OUTSIDE_SUBSCRIPTIONS;
    assert_eq!(heard, [FileWatchMessage::Refused { key: format!("i{last}"), reason: RefusedReason::Limit }]);
    let failed = page.failed(&subject(File, "/tmp/0.log"), "too many watches");
    assert_eq!(
        failed,
        [Effect::Page(FileWatchMessage::Unavailable { key: "o0".into(), reason: "too many watches".into() })]
    );
    // Its place is free again.
    let (held, _, heard) = split(page.subscribe(vec![asked("again", File, "/tmp/again.log")], now));
    assert_eq!(held, [subject(File, "/tmp/again.log")]);
    assert!(heard.is_empty());
}

/// A renew answers the keys the page holds and renews their subjects with
/// the working tree's own; a key the page stops renewing is let go once its
/// lease runs out, as the page counts it over already.
#[test]
fn a_renew_renews_what_the_page_holds_and_a_key_not_renewed_lapses() {
    use SubscriptionKind::*;
    let start = Instant::now();
    let mut page = PageSubscriptions::new(Some("/work".into()));
    page.subscribe(vec![asked("kept", File, "/work/a.rs"), asked("left", Head, "/work")], start);
    let later = start + Duration::from_secs(4 * 60);
    let renewed = page.renew(&["kept".into(), "unknown".into()], later);
    assert_eq!(
        renewed,
        [
            Effect::Page(FileWatchMessage::Renewed { keys: vec!["kept".into()] }),
            Effect::Renew(vec![subject(File, "/work/a.rs"), subject(Worktree, "/work")]),
        ]
    );
    assert_eq!(page.next_expiry(), Some(start + wire::SUBSCRIPTION_LEASE));
    let (_, released, _) = split(page.expire(start + wire::SUBSCRIPTION_LEASE));
    assert_eq!(released, [subject(Head, "/work")]);
    assert_eq!(page.next_expiry(), Some(later + wire::SUBSCRIPTION_LEASE));
}

/// A `changed` too large for a channel's message goes in as many as it
/// takes, every key once and in order.
#[test]
fn a_changed_too_large_for_one_message_is_split_without_losing_a_key() {
    let changes: Vec<(String, u64)> = (0..1000).map(|index| (format!("key-{index}"), index)).collect();
    let texts = FileWatchMessage::Changed { changes: changes.clone() }.into_texts(2048);
    assert!(texts.len() > 1);
    let mut heard = Vec::new();
    for text in texts {
        assert!(text.len() <= 2048, "{}", text.len());
        let FileWatchMessage::Changed { changes } = serde_json::from_str(&text).unwrap() else {
            panic!("{text}");
        };
        heard.extend(changes);
    }
    assert_eq!(heard, changes);
}

/// An installation's directory is named after its backend's host, then its
/// port when it is not the scheme's default, then its path's segments, and
/// last four hex digits of the SHA-256 of the whole URL. The readable part is
/// a file name on every system: an IPv6 host's colons and a percent-encoded
/// byte are replaced, and a very long path is cut.
#[test]
fn an_installation_is_named_after_its_backends_host_port_and_path() {
    let cases = [
        ("https://demi.example.com/", "demi.example.com-748c"),
        ("https://demi.example.com:443/", "demi.example.com-748c"),
        ("https://demi.example.com", "demi.example.com-748c"),
        ("http://127.0.0.1:3310/", "127.0.0.1-3310-825d"),
        ("https://example.com/team/a/", "example.com-team-a-"),
        ("http://[::1]:3310/", "__1-3310-e0f6"),
        ("https://[2001:db8::7]/", "2001_db8__7-59e2"),
        ("https://example.com/a%20b", "example.com-a_20b-f785"),
        ("https://example.com/what*", "example.com-what_-"),
    ];
    for (url, expected) in cases {
        let name = installation_name(&url::Url::parse(url).unwrap());
        assert!(name.starts_with(expected), "{url}: {name}");
        assert!(
            name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_')),
            "{name}"
        );
    }
}

/// Backends that read alike have names of their own, so that each installs on
/// one machine beside the others; the name of a backend does not depend on how
/// its URL is spelled where the spelling means the same.
#[test]
fn backends_that_read_alike_have_installations_of_their_own() {
    let pairs = [
        ("http://example.com/", "https://example.com/"),
        ("https://example.com/team-a", "https://example.com/team/a"),
        ("http://example.com:3310/", "http://example.com/3310"),
        ("https://example.com/a%20b", "https://example.com/a_20b"),
        ("https://example.com/Team", "https://example.com/team"),
    ];
    for (first, second) in pairs {
        let first_name = installation_name(&url::Url::parse(first).unwrap());
        let second_name = installation_name(&url::Url::parse(second).unwrap());
        assert_ne!(first_name, second_name, "{first} and {second}");
        // A case-insensitive file system tells them apart too.
        assert_ne!(first_name.to_lowercase(), second_name.to_lowercase(), "{first} and {second}");
    }
}

/// A legal but very long path does not make a name a file system refuses: the
/// readable part is cut, and the hash still tells the backends apart.
#[test]
fn a_very_long_backend_path_still_names_an_installable_directory() {
    let long = |tail: &str| format!("https://example.com/{}{tail}", "a".repeat(250));
    let first = installation_name(&url::Url::parse(&long("x")).unwrap());
    let second = installation_name(&url::Url::parse(&long("y")).unwrap());
    assert!(first.len() <= 101 && second.len() <= 101, "{first}");
    assert_ne!(first, second);
    assert!(first.starts_with("example.com-aaaa"));
}

/// A conversation's scratch directory is named by the first 12 hex digits of
/// its id.
#[test]
fn a_scratch_directory_is_named_by_twelve_digits_of_the_conversation_id() {
    assert_eq!(
        demi_command_protocol::scratch_name("5b2c9e41-7a3d-4f08-9c6e-1d8b3a7f2e90"),
        "5b2c9e417a3d"
    );
    assert_eq!(demi_command_protocol::scratch_name("5b2c9e41"), "5b2c9e41");
}
