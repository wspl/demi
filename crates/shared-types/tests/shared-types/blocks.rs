//! Transcript blocks as the conversation database stores them and the page
//! receives them: `fixtures/blocks.json` holds one of each kind in the wire
//! shape, and every block decodes and encodes to exactly that JSON.

use demi_shared_types::{Block, DecodeError, decode};
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

/// The block a case of `fixtures/blocks-mutations.json` describes: its
/// fixture, by id, with the fields it names removed and set.
fn mutated(case: &Value) -> Value {
    let mut value = fixture(case["fixture"].as_str().unwrap());
    for pointer in case["remove"].as_array().into_iter().flatten() {
        value = without(value, pointer.as_str().unwrap());
    }
    for (pointer, field) in case["set"].as_object().into_iter().flatten() {
        value = with(value, pointer, field.clone());
    }
    value
}

/// The cases the browser's generated schemas are checked with too; each
/// says whether the browser refuses it as well (`contracts.md` § Strict and
/// tolerant objects, § Rules only Rust checks).
#[test]
fn stored_blocks_refuse_what_the_contract_does_not_hold() {
    let table: Value = serde_json::from_str(include_str!("fixtures/blocks-mutations.json")).unwrap();
    for case in table["refused"].as_array().unwrap() {
        let value = mutated(case);
        assert!(decode_value(&value).is_err(), "{} was accepted: {value}", case["why"]);
    }
}

#[test]
fn a_rule_break_names_the_field() {
    let value = with(fixture("u1"), "/content/6/name", json!(""));
    let error = decode_value(&value).unwrap_err();
    assert!(matches!(error, DecodeError::Invalid(_)), "{error}");
    assert!(error.to_string().contains("content[6][0].name"), "{error}");
    let value = with(fixture("m1"), "/id", json!("m2"));
    let error = decode_value(&value).unwrap_err().to_string();
    assert!(error.contains("must be the message's id, m1"), "{error}");
}
