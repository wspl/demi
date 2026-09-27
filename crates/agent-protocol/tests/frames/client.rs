//! What the browser sends: `fixtures/client-frames.json` holds every frame in
//! its wire shape.

use demi_agent_protocol::{ClientFrame, FrameError, decode_client_frame};
use serde_json::{Value, json};

fn fixtures() -> Vec<Value> {
    serde_json::from_str(include_str!("fixtures/client-frames.json")).expect("the fixture is JSON")
}

fn fixture(kind: &str) -> Value {
    fixtures()
        .into_iter()
        .find(|fixture| fixture["type"] == kind)
        .unwrap_or_else(|| panic!("no {kind} frame"))
}

#[test]
fn every_client_frame_decodes_from_its_wire_shape() {
    let mut kinds = Vec::new();
    for fixture in fixtures() {
        let frame = decode_client_frame(&fixture.to_string())
            .unwrap_or_else(|error| panic!("{fixture}: {error}"));
        assert_eq!(serde_json::to_value(&frame).unwrap(), fixture);
        assert_eq!(frame.kind().to_string(), fixture["type"].as_str().unwrap());
        kinds.push(frame.kind());
    }
    kinds.dedup();
    assert_eq!(kinds.len(), 19, "one fixture of every frame");
}

/// `value` with the field at `pointer` set to `field`, adding it if absent.
fn with(mut value: Value, pointer: &str, field: Value) -> Value {
    let (parent, key) = pointer.rsplit_once('/').unwrap();
    match value.pointer_mut(parent).unwrap_or_else(|| panic!("{parent}")) {
        Value::Object(object) => {
            object.insert(key.to_owned(), field);
        }
        Value::Array(array) => array[key.parse::<usize>().unwrap()] = field,
        other => panic!("{pointer} is inside {other}"),
    }
    value
}

/// The frame a case of `fixtures/client-frames-mutations.json` describes:
/// its value, or its fixture, by type, with the fields it names removed and
/// set.
fn mutated(case: &Value) -> Value {
    if let Some(value) = case.get("value") {
        return value.clone();
    }
    let mut value = fixture(case["fixture"].as_str().unwrap());
    for pointer in case["remove"].as_array().into_iter().flatten() {
        let (parent, key) = pointer.as_str().unwrap().rsplit_once('/').unwrap();
        let object = value.pointer_mut(parent).unwrap().as_object_mut().unwrap();
        assert!(object.remove(key).is_some(), "{pointer} is absent");
    }
    for (pointer, field) in case["set"].as_object().into_iter().flatten() {
        value = with(value, pointer, field.clone());
    }
    value
}

fn mutations(kind: &str) -> Vec<(String, Value)> {
    let table: Value = serde_json::from_str(include_str!("fixtures/client-frames-mutations.json")).unwrap();
    table[kind]
        .as_array()
        .unwrap()
        .iter()
        .map(|case| (case["why"].as_str().unwrap().to_owned(), mutated(case)))
        .collect()
}

/// The cases the browser's generated schemas are checked with too; each
/// refused one says whether the browser's schema refuses it as well.
#[test]
fn client_frames_refuse_unknown_fields_nulls_and_values_outside_their_bounds() {
    for (why, frame) in mutations("refused") {
        match decode_client_frame(&frame.to_string()) {
            Err(FrameError::Invalid(_)) => {}
            other => panic!("{why}: {other:?} for {frame}"),
        }
    }
}

/// A file name counts Unicode scalar values and may start with dots.
#[test]
fn client_frames_accept_values_at_their_bounds() {
    for (why, frame) in mutations("accepted") {
        assert!(decode_client_frame(&frame.to_string()).is_ok(), "{why}: {frame}");
    }
}

#[test]
fn a_message_that_is_not_json_is_told_from_an_invalid_frame() {
    for text in ["not json", "{\"type\":", ""] {
        assert!(matches!(decode_client_frame(text), Err(FrameError::NotJson(_))), "{text:?}");
    }
    for text in ["[1, 2]", "{\"type\": \"nope\"}", "null"] {
        assert!(matches!(decode_client_frame(text), Err(FrameError::Invalid(_))), "{text:?}");
    }
    let error = decode_client_frame(&with(fixture("send"), "/content/2/fileName", json!("..")).to_string())
        .unwrap_err()
        .to_string();
    assert!(error.contains("content[2].file_name"), "{error}");
    let error = decode_client_frame(&with(fixture("send"), "/content/0", json!({"type": "attachment", "path": "/a"})).to_string())
        .unwrap_err()
        .to_string();
    assert!(error.contains("content[0]"), "{error}");
}

#[test]
fn client_frames_are_strict_in_their_schema() {
    let schema = serde_json::to_value(schemars::schema_for!(ClientFrame)).unwrap();
    for variant in schema["oneOf"].as_array().unwrap() {
        assert_eq!(variant["additionalProperties"], false, "{}", variant["properties"]["type"]);
    }
}
