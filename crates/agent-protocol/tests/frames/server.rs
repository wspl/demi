//! What the backend sends: `fixtures/server-frames.json` holds every frame in
//! the shape the page receives.

use demi_agent_protocol::ServerFrame;
use serde_json::{Value, json};

fn fixtures() -> Vec<Value> {
    serde_json::from_str(include_str!("fixtures/server-frames.json")).expect("the fixture is JSON")
}

fn decode(value: &Value) -> ServerFrame {
    demi_core::decode(&value.to_string()).unwrap_or_else(|error| panic!("{value}: {error}"))
}

#[test]
fn every_server_frame_keeps_its_wire_shape() {
    let mut kinds: Vec<String> = Vec::new();
    for fixture in fixtures() {
        let frame = decode(&fixture);
        assert_eq!(serde_json::to_value(&frame).unwrap(), fixture);
        kinds.push(fixture["type"].as_str().unwrap().to_owned());
    }
    kinds.dedup();
    assert_eq!(kinds.len(), 18, "one fixture of every frame");
}

#[test]
fn a_page_accepts_frame_fields_it_does_not_know() {
    let frame = json!({"type": "phase", "phase": "idle", "since": "2026-09-21T14:13:20.000Z"});
    assert_eq!(decode(&frame), ServerFrame::Phase { phase: demi_core::SessionPhase::Idle });

    let schema = serde_json::to_value(schemars::schema_for!(ServerFrame)).unwrap();
    for variant in schema["oneOf"].as_array().unwrap() {
        assert!(variant.get("additionalProperties").is_none(), "{}", variant["properties"]["type"]);
    }
}
