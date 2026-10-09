//! What the backend sends: `fixtures/server-frames.json` holds every frame in
//! the shape the page receives.

use demi_conversation_socket_protocol::{ServerFrame, TranscriptPatch};
use demi_shared_types::Block;
use serde_json::{Value, json};

fn fixtures() -> Vec<Value> {
    serde_json::from_str(include_str!("fixtures/server-frames.json")).expect("the fixture is JSON")
}

fn decode(value: &Value) -> ServerFrame {
    demi_shared_types::decode(&value.to_string()).unwrap_or_else(|error| panic!("{value}: {error}"))
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
    assert_eq!(kinds.len(), 20, "one fixture of every frame");
}

#[test]
fn a_page_accepts_frame_fields_it_does_not_know() {
    let frame = json!({"type": "phase", "phase": "idle", "since": "2026-09-21T14:13:20.000Z"});
    assert_eq!(
        decode(&frame),
        ServerFrame::Phase {
            phase: demi_shared_types::SessionPhase::Idle
        }
    );

    let schema = serde_json::to_value(schemars::schema_for!(ServerFrame)).unwrap();
    for variant in schema["oneOf"].as_array().unwrap() {
        assert!(
            variant.get("additionalProperties").is_none(),
            "{}",
            variant["properties"]["type"]
        );
    }
}

/// Every block a frame carries.
fn blocks(frame: &mut ServerFrame) -> Vec<&mut Block> {
    match frame {
        ServerFrame::TranscriptReset { blocks, .. }
        | ServerFrame::SubagentTranscriptReset { blocks, .. } => blocks.iter_mut().collect(),
        ServerFrame::TranscriptPatch { patches, .. }
        | ServerFrame::SubagentTranscriptPatch { patches, .. } => patched(patches),
        _ => Vec::new(),
    }
}

/// Every block `patches` carry.
fn patched(patches: &mut [TranscriptPatch]) -> Vec<&mut Block> {
    patches
        .iter_mut()
        .flat_map(|patch| match patch {
            TranscriptPatch::Add { value, .. } | TranscriptPatch::ReplaceBlock { value, .. } => {
                vec![value]
            }
            TranscriptPatch::Replace { value } => value.iter_mut().collect(),
            TranscriptPatch::AppendText { .. } => Vec::new(),
        })
        .collect()
}

// The entries a provider keeps on a block never leave the backend
// (`claude-code.md` § The session a process resumes): a frame that carries
// blocks holding them has the wire shape of one whose blocks hold none.
#[test]
fn a_block_reaches_the_page_without_the_entries_its_provider_kept_on_it() {
    let mut holding = 0;
    for fixture in fixtures() {
        let mut frame = decode(&fixture);
        for block in blocks(&mut frame) {
            if let Some(entries) = block.entries_mut() {
                entries.push(json!({ "type": "assistant", "uuid": "kept" }));
                holding += 1;
            }
        }
        assert_eq!(serde_json::to_value(&frame).unwrap(), fixture);
    }
    // A reset, an added, a replaced and a rewritten block, and a
    // subagent's reset.
    assert_eq!(holding, 5);
}
