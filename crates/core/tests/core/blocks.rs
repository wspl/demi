//! Transcript blocks as the conversation database stores them and the page
//! receives them: `fixtures/blocks.json` holds one of each kind in the wire
//! shape, and every block decodes and encodes to exactly that JSON.

use demi_core::{Block, DecodeError, ToolView, decode};
use serde_json::{Value, json};

fn fixtures() -> Vec<Value> {
    serde_json::from_str(include_str!("fixtures/blocks.json")).expect("the fixture is JSON")
}

fn decode_value(value: &Value) -> Result<Block, DecodeError> {
    decode(&value.to_string())
}

#[test]
fn every_block_kind_keeps_its_wire_shape() {
    let fixtures = fixtures();
    for fixture in &fixtures {
        let block = decode_value(fixture).unwrap_or_else(|error| panic!("{fixture}: {error}"));
        let encoded = serde_json::to_value(&block).unwrap();
        assert_eq!(&encoded, fixture);
    }
    let kinds: Vec<&str> = fixtures
        .iter()
        .map(|fixture| fixture["type"].as_str().unwrap())
        .collect();
    for kind in [
        "user",
        "context",
        "wakeup",
        "steer",
        "agent_message",
        "resume",
        "abort",
        "thinking",
        "redacted_thinking",
        "text",
        "tool_call",
        "response",
        "error",
        "compaction_boundary",
        "compaction_marker",
    ] {
        assert!(kinds.contains(&kind), "the fixture has no {kind} block");
    }
}

#[test]
fn every_block_names_its_id_time_and_model_and_only_a_user_block_is_editable() {
    for fixture in fixtures() {
        let block = decode_value(&fixture).unwrap();
        assert_eq!(block.id().as_str(), fixture["id"].as_str().unwrap());
        assert_eq!(block.created_at().to_string(), fixture["createdAt"].as_str().unwrap());
        assert_eq!(block.model().provider_id, fixture["model"]["providerId"].as_str().unwrap());
        assert_eq!(block.is_editable(), fixture["type"] == "user");
    }
}

#[test]
fn a_tool_call_view_is_typed_by_its_kind() {
    let fixtures = fixtures();
    let views: Vec<Option<ToolView>> = fixtures
        .iter()
        .filter(|fixture| fixture["type"] == "tool_call")
        .map(|fixture| match decode_value(fixture).unwrap() {
            Block::ToolCall(call) => call.view,
            _ => unreachable!(),
        })
        .collect();
    assert!(views[0].is_none());
    assert!(matches!(&views[1], Some(ToolView::Shell(view)) if view.files.as_ref().is_some_and(|files| files.len() == 2)));
    assert!(matches!(views[2], Some(ToolView::RepeatedShellExec { count: 7, .. })));
    assert!(matches!(views[4], Some(ToolView::YieldWakeup { duration_ms: 120_000, .. })));
}

/// The fixture whose `id` is `id`.
fn fixture(id: &str) -> Value {
    fixtures()
        .into_iter()
        .find(|fixture| fixture["id"] == id)
        .unwrap_or_else(|| panic!("no fixture {id}"))
}

/// `value` with the field at `pointer` set to `field`, adding it if absent.
fn with(mut value: Value, pointer: &str, field: Value) -> Value {
    let (parent, key) = pointer.rsplit_once('/').unwrap();
    let target = value.pointer_mut(parent).unwrap_or_else(|| panic!("{parent}"));
    match target {
        Value::Object(object) => {
            object.insert(key.to_owned(), field);
        }
        Value::Array(array) => array[key.parse::<usize>().unwrap()] = field,
        other => panic!("{pointer} is inside {other}"),
    }
    value
}

/// `value` without the field at `pointer`.
fn without(mut value: Value, pointer: &str) -> Value {
    let (parent, key) = pointer.rsplit_once('/').unwrap();
    let object = value.pointer_mut(parent).unwrap().as_object_mut().unwrap();
    assert!(object.remove(key).is_some(), "{pointer} is absent");
    value
}

#[test]
fn stored_blocks_refuse_what_the_contract_does_not_hold() {
    let completion = "subagent:child-1:1790000000000";
    let refused = [
        ("an unknown field", with(fixture("u1"), "/unknown", json!(1))),
        ("a hidden flag", with(fixture("u1"), "/hidden", json!(true))),
        ("a resolved content field", with(fixture("u1"), "/resolvedContent", json!([]))),
        ("a nullable field that is absent", without(fixture("u1"), "/preamble")),
        ("an optional field that is null", with(fixture("u1"), "/content/8/snippet", Value::Null)),
        ("a block kind the transcript does not have", with(fixture("r1"), "/type", json!("extension_state_snapshot"))),
        ("an empty id", with(fixture("u1"), "/id", json!(""))),
        ("a time that is not RFC 3339", with(fixture("u1"), "/createdAt", json!("yesterday"))),
        ("a time finer than a millisecond", with(fixture("u1"), "/createdAt", json!("2026-09-21T14:13:20.000123Z"))),
        ("client content in a stored block", with(fixture("u1"), "/content/0", json!({"type": "upload", "ref": "a1", "fileName": "a.txt"}))),
        ("an untagged media source", with(fixture("u1"), "/content/1/source", json!({"mediaType": "image/png", "data": "iVBORw0KGgo="}))),
        ("bytes that are not base64", with(fixture("u1"), "/content/1/source/data", json!("not base64!"))),
        ("a blob reference that is not a SHA-256", with(fixture("u1"), "/content/2/source/ref", json!("sha-1-2-3"))),
        ("a blob reference that is not a string", with(fixture("u1"), "/content/2/source/ref", json!(42))),
        ("a document source without its tag", with(fixture("u1"), "/content/5/source", json!({"data": "JVBERi0xLjc=", "mediaType": "application/pdf", "fileName": "spec.pdf"}))),
        ("an attachment without a name", with(fixture("u1"), "/content/8/name", json!(""))),
        ("an attachment hash that is not a SHA-256", with(fixture("u1"), "/content/8/sha256", json!("abc"))),
        ("a size beyond JavaScript's safe integers", with(fixture("u1"), "/content/8/sizeBytes", json!(9_007_199_254_740_992_u64))),
        ("an output limit of zero", with(fixture("u1"), "/model/model/outputLimit", json!(0))),
        ("an output limit that is not whole", with(fixture("u1"), "/model/model/outputLimit", json!(1.5))),
        ("an extension outside the native set", with(fixture("u1"), "/model/model/acceptedExtensions/0", json!("txt"))),
        ("a selection without its service tier", without(fixture("u1"), "/model/serviceTierId")),
        ("a field on a capability that has none", with(fixture("u1"), "/model/model/thinking/3/extra", json!(1))),
        ("an effort setting without its summary", with(fixture("u1"), "/model/thinking", json!({"type": "effort", "effort": "high"}))),
        ("a receipt whose block is not its message's", with(fixture("m1"), "/id", json!("m2"))),
        ("an explicit agent message that is blank", with(fixture("m1"), "/message/content", json!(" \u{feff}\n"))),
        ("a round that is negative", with(fixture("m1"), "/message/sender/round", json!(-1))),
        ("a completion whose id names another round", with(with(fixture(completion), "/id", json!("subagent:child-1:1")), "/message/id", json!("subagent:child-1:1"))),
        ("a forkable flag that is null", with(fixture("t1"), "/forkable", Value::Null)),
        ("a streaming output field", with(fixture("tc1"), "/streamingOutput", json!([]))),
        ("a tool call status the transcript does not have", with(fixture("tc1"), "/status", json!("running"))),
        ("a view kind no tool has", with(fixture("tc2"), "/view/kind", json!("tool_error"))),
        ("an exit code that is null", with(fixture("tc2"), "/view/exitCode", Value::Null)),
        ("an edited file without segments", with(fixture("tc2"), "/view/files/0/edits", json!([]))),
        ("an unknown view field", with(fixture("tc2"), "/view/unknown", json!(1))),
        ("a negative repeat count", with(fixture("tc3"), "/view/count", json!(-1))),
        ("diagnostics that are null", with(fixture("e1"), "/diagnostics", Value::Null)),
        ("a failure source outside the set", with(fixture("e1"), "/diagnostics/source", json!("vendor"))),
        ("an error without its code", without(fixture("e2"), "/code")),
        ("a marker without its boundary", with(fixture("cm1"), "/boundaryId", json!(""))),
    ];
    for (why, value) in refused {
        assert!(decode_value(&value).is_err(), "{why} was accepted: {value}");
    }
}

#[test]
fn a_rule_break_names_the_field() {
    let value = with(fixture("u1"), "/content/8/name", json!(""));
    let error = decode_value(&value).unwrap_err();
    assert!(matches!(error, DecodeError::Invalid(_)), "{error}");
    assert!(error.to_string().contains("content[8][0].name"), "{error}");
    let value = with(fixture("m1"), "/id", json!("m2"));
    let error = decode_value(&value).unwrap_err().to_string();
    assert!(error.contains("must be the message's id, m1"), "{error}");
}
