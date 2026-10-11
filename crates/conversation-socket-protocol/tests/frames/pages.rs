//! Which blocks a page of a transcript holds (`web-api.md` § Pages): whole
//! requests within the page budget, from where the page is taken, in their
//! light form.

use demi_conversation_socket_protocol::{PAGE_BYTES, PageAt, light, page, requests};
use demi_shared_types::{Block, ToolView};
use serde_json::{Value, json};

// Cost: pure, under a millisecond.

#[test]
fn requests_start_at_user_blocks_and_the_blocks_before_the_first_join_it() {
    assert_eq!(requests(0, &[]), Vec::<std::ops::Range<usize>>::new());
    assert_eq!(requests(5, &[]), [0..5]);
    assert_eq!(requests(6, &[0, 2, 5]), [0..2, 2..5, 5..6]);
    // A subagent's brief may follow a context block.
    assert_eq!(requests(6, &[1, 4]), [0..4, 4..6]);
}

#[test]
fn a_page_holds_whole_requests_within_the_budget_and_at_least_one() {
    // Five requests of ten blocks each; `size` gives each request's bytes.
    let five = requests(50, &[0, 10, 20, 30, 40]);
    let third = PAGE_BYTES / 3;
    let even = |_: std::ops::Range<usize>| third;
    let cases: &[(&str, PageAt, &dyn Fn(std::ops::Range<usize>) -> usize, std::ops::Range<usize>)] = &[
        ("the latest takes requests back while they fit", PageAt::Latest, &even, 20..50),
        ("before an edge, the requests that end there", PageAt::Before(30), &even, 0..30),
        ("before an edge inside a request, that request whole", PageAt::Before(25), &even, 0..30),
        ("after an edge, the requests that start there", PageAt::After(9), &even, 10..40),
        ("around a block, its request, then after it, then before it", PageAt::Around(22), &even, 10..40),
        ("around the last block, the requests before it", PageAt::Around(49), &even, 20..50),
        ("a request past the budget alone", PageAt::Latest, &|_| PAGE_BYTES * 2, 40..50),
        ("nothing before the start", PageAt::Before(0), &even, 0..0),
        ("nothing after the end", PageAt::After(49), &even, 50..50),
    ];
    for (name, at, size, expected) in cases {
        assert_eq!(page(&five, *at, size), *expected, "{name}");
    }
    assert_eq!(page(&[], PageAt::Latest, |_| 0), 0..0, "an empty transcript");
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
        "status": "completed", "output": [],
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

fn thinking(id: &str, text: &str, signature: Option<&str>) -> Value {
    json!({
        "type": "thinking", "id": id, "createdAt": "2026-10-01T00:00:00.000Z",
        "model": model(), "text": text, "signature": signature
    })
}

#[test]
fn a_pages_thinking_carries_its_texts_length_in_place_of_its_text() {
    // Thinking with text, and the textless signed thinking Gemini writes
    // before a call, which the page hides as it does live.
    let cases = [
        ("text", thinking("k1", "Käse 🧀 first.", Some("anthropic:sig")), 13),
        ("no text", thinking("k2", "", Some("google:sig")), 0),
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
