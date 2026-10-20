//! Which blocks a page of a transcript holds: blocks within the page budget,
//! from where the page is taken, wherever requests begin and end, in their
//! light form.

use demi_conversation_socket_protocol::{PAGE_BYTES, PageAt, SIZED_AT_ONCE, light, page};
use demi_shared_types::{Block, ToolView};
use serde_json::{Value, json};

// Cost: pure, under a millisecond.

#[test]
fn a_page_holds_blocks_within_the_budget_and_at_least_one() {
    // Fifty blocks; a tenth of the budget each, unless a case says otherwise.
    let tenth = PAGE_BYTES / 10;
    let even = |range: std::ops::Range<usize>| Some(vec![tenth; range.len()]);
    let cases: &[(&str, PageAt, std::ops::Range<usize>)] = &[
        ("the latest takes blocks back while they fit", PageAt::Latest, 40..50),
        ("before an edge, the blocks that end there", PageAt::Before(30), 20..30),
        ("after an edge, the blocks that start there", PageAt::After(9), 10..20),
        ("around a block, then after it, then before it", PageAt::Around(22), 18..28),
        ("around the last block, the blocks before it", PageAt::Around(49), 40..50),
        ("nothing before the start", PageAt::Before(0), 0..0),
        ("nothing after the end", PageAt::After(49), 50..50),
    ];
    for (name, at, expected) in cases {
        assert_eq!(page(50, *at, even), *expected, "{name}");
    }
    // One block past the budget is a page alone; a long request is several.
    let large = |range: std::ops::Range<usize>| Some(vec![PAGE_BYTES * 2; range.len()]);
    assert_eq!(page(50, PageAt::Latest, large), 49..50, "a block past the budget alone");
    assert_eq!(page(0, PageAt::Latest, even), 0..0, "an empty transcript");
}

#[test]
fn a_page_sizes_a_few_blocks_at_a_time_however_long_the_transcript() {
    // A page read from storage decodes only the blocks it sizes.
    let mut sized = 0;
    let range = page(100_000, PageAt::Latest, |range| {
        assert!(range.len() <= SIZED_AT_ONCE);
        sized += range.len();
        Some(vec![PAGE_BYTES / 100; range.len()])
    });
    assert_eq!(range, 100_000 - 100..100_000);
    assert!(sized <= 100 + SIZED_AT_ONCE, "{sized} blocks sized");
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
