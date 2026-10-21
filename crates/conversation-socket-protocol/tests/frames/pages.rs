//! Which blocks a page of a transcript holds: a thousand of them from where the
//! page is taken, wherever requests begin and end, in their light form.

use demi_conversation_socket_protocol::{PAGE_BLOCKS, PageAt, light, page};
use demi_shared_types::{Block, ToolView};
use serde_json::{Value, json};

// Cost: pure, under a millisecond.

#[test]
fn a_page_is_a_thousand_blocks_wherever_it_is_taken() {
    let length = 5_000;
    let cases: &[(&str, PageAt, std::ops::Range<usize>)] = &[
        ("the latest ends at the end", PageAt::Latest, 4_000..5_000),
        ("before a block, the blocks that end there", PageAt::Before(3_000), 2_000..3_000),
        ("after a block, the blocks that start after it", PageAt::After(999), 1_000..2_000),
        ("around a block, half on each side", PageAt::Around(2_500), 2_000..3_000),
    ];
    for (name, at, expected) in cases {
        assert_eq!(page(length, *at), *expected, "{name}");
        assert_eq!(expected.len(), PAGE_BLOCKS, "{name}");
    }
}

#[test]
fn only_the_first_and_the_last_page_of_a_transcript_are_shorter() {
    let cases: &[(&str, usize, PageAt, std::ops::Range<usize>)] = &[
        ("a short transcript is one page", 50, PageAt::Latest, 0..50),
        ("before a block near the start, the first page", 5_000, PageAt::Before(300), 0..300),
        ("after a block near the end, the last page", 5_000, PageAt::After(4_700), 4_701..5_000),
        ("around a block near the start, a full page from the start", 5_000, PageAt::Around(10), 0..1_000),
        ("around a block near the end, a full page to the end", 5_000, PageAt::Around(4_990), 4_000..5_000),
        ("around the only block of a short transcript", 1, PageAt::Around(0), 0..1),
        ("nothing before the start", 5_000, PageAt::Before(0), 0..0),
        ("nothing after the end", 5_000, PageAt::After(4_999), 5_000..5_000),
        ("an empty transcript", 0, PageAt::Latest, 0..0),
        ("an index past the end answers an empty page there", 50, PageAt::After(900), 50..50),
    ];
    for (name, length, at, expected) in cases {
        assert_eq!(page(*length, *at), *expected, "{name}");
    }
}

fn model() -> Value {
    json!({
        "providerId": "anthropic",
        "model": {
            "id": "claude-sonnet-4-5",
            "name": "Claude Sonnet 4.5",
            "contextWindow": 200000,
            "outputLimit": 64000,
            "thinking": [{"type": "disabled"}],
            "acceptedExtensions": []
        },
        "thinking": {"type": "disabled"},
        "serviceTierId": null
    })
}

fn user(id: &str) -> Value {
    json!({
        "type": "user", "id": id, "turnId": id, "createdAt": "2026-10-01T00:00:00.000Z",
        "model": model(), "content": [{"type": "text", "text": "Go"}], "preamble": null
    })
}

fn text(id: &str) -> Value {
    json!({
        "type": "text", "id": id, "createdAt": "2026-10-01T00:00:00.000Z",
        "model": model(), "text": "Done."
    })
}

/// A shell call of the run `run` whose lists each name one file after it.
fn call(id: &str, run: &str) -> Value {
    let list = json!([{ "path": format!("/w/{id}"), "kind": "modified", "lines": {"added": 1, "removed": 0}, "edits": [] }]);
    json!({
        "type": "tool_call", "id": id, "run": run, "createdAt": "2026-10-01T00:00:00.000Z",
        "model": model(), "toolUseId": id, "toolName": "shell", "input": "{}",
        "status": "completed", "output": [], "endedAt": "2026-10-01T00:00:00.001Z",
        "view": {
            "kind": "shell", "status": "exited", "commandId": id, "exitCode": 0,
            "runningMs": 1, "idleMs": 0, "chunks": [], "viewTruncated": false,
            "runFiles": list, "requestFiles": list
        }
    })
}

/// Which of the lists each call keeps: its run's and its request's.
fn kept(blocks: &[Block]) -> Vec<(String, bool, bool)> {
    blocks
        .iter()
        .filter_map(|block| match block {
            Block::ToolCall(call) => match &call.view {
                Some(ToolView::Shell(view)) => Some((
                    call.id.to_string(),
                    view.run_files.is_some(),
                    view.request_files.is_some(),
                )),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

#[test]
fn a_pages_light_form_keeps_the_lists_of_the_latest_call_of_each_run_and_request() {
    // Two runs in the first request, one in the second.
    let blocks: Vec<Block> = [
        user("u1"),
        call("c1", "c1"),
        call("c2", "c1"),
        text("x1"),
        call("c3", "c3"),
        text("x2"),
        user("u2"),
        call("c4", "c4"),
    ]
    .into_iter()
    .map(|block| demi_shared_types::decode_value(block).unwrap())
    .collect();
    let light = light(&blocks);
    assert_eq!(
        kept(&light),
        [
            ("c1".to_owned(), false, false),
            ("c2".to_owned(), true, false),
            ("c3".to_owned(), true, true),
            ("c4".to_owned(), true, true),
        ]
    );
    // What a page shows of each run and request is what the whole blocks
    // carry.
    for (light, whole) in light.iter().zip(&blocks) {
        if let (Block::ToolCall(light), Block::ToolCall(whole)) = (light, whole)
            && let (Some(ToolView::Shell(light)), Some(ToolView::Shell(whole))) = (&light.view, &whole.view)
            && light.run_files.is_some()
        {
            assert_eq!(light.run_files, whole.run_files);
        }
    }
}

/// A message from the agent `sender`, in the run `run` when it is a step.
fn message(id: &str, sender: &str, run: Option<&str>) -> Value {
    let mut block = json!({
        "type": "agent_message", "id": id, "turnId": id, "createdAt": "2026-10-01T00:00:00.000Z",
        "model": model(),
        "message": {
            "id": id, "sender": {"id": sender, "number": 1, "description": "Tidy the notes", "round": 1},
            "recipientId": "notes", "timestamp": "2026-10-01T00:00:00.000Z",
            "content": "Keep the old heading.", "event": {"type": "message"}
        }
    });
    if let Some(run) = run {
        block["run"] = json!(run);
    }
    block
}

#[test]
fn a_pages_light_form_keeps_the_words_of_a_message_that_shows_as_a_bubble() {
    // The parent's message ended the child's run and shows its words; the
    // child's own child's message is a step, whose row shows none.
    let blocks: Vec<Block> = [user("u1"), call("c1", "c1"), message("m1", "root", None), message("m2", "links", Some("m2"))]
        .into_iter()
        .map(|block| demi_shared_types::decode_value(block).unwrap())
        .collect();
    let words: Vec<String> = light(&blocks)
        .into_iter()
        .filter_map(|block| match block {
            Block::AgentMessage(message) => Some(message.message.content),
            _ => None,
        })
        .collect();
    assert_eq!(words, ["Keep the old heading.", ""]);
}

fn thinking(id: &str, text: &str, signature: Option<&str>) -> Value {
    json!({
        "type": "thinking", "id": id, "createdAt": "2026-10-01T00:00:00.000Z",
        "model": model(), "text": text, "signature": signature
    })
}

#[test]
fn a_pages_thinking_carries_its_texts_length_in_place_of_its_text() {
    // Thinking with text, counted without the white space around it; the
    // textless signed thinking Gemini writes before a call, and a thinking
    // of white space only, which the page hides as it does live.
    let cases = [
        ("text", thinking("k1", "\n Käse 🧀 first.\n", Some("anthropic:sig")), 13),
        ("no text", thinking("k2", "", Some("google:sig")), 0),
        ("white space only", thinking("k3", " \n\u{3000}", None), 0),
    ];
    for (name, whole, length) in cases {
        let block: Block = demi_shared_types::decode_value(whole.clone()).unwrap();
        let light = serde_json::to_value(&light(std::slice::from_ref(&block))[0]).unwrap();
        let mut expected = whole.clone();
        expected["text"] = json!("");
        expected["signature"] = Value::Null;
        expected["textLength"] = json!(length);
        assert_eq!(light, expected, "{name}");
        // The block whole, as a row opened reads it, carries no length.
        assert_eq!(serde_json::to_value(&block).unwrap(), whole, "{name}");
    }
}
