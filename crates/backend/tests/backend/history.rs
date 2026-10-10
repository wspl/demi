//! A conversation's history read a page at a time (`web-api.md`
//! § Conversation history): pages of whole requests from the latest back, a
//! page around a block, the refusal of a page whose edge moved, the light
//! form and a block read whole.

use demi_provider_common::testing::MockVendor;
use demi_shared_types::Block;
use demi_web_api_protocol::conversations::{TranscriptPage, WholeBlock};
use demi_web_api_protocol::error::ErrorCode;
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{FIRST, Socket, anthropic, answer, choose, create, kinds, tool_use, transcript};
use crate::support::{Harness, Session, TestBackend};

async fn page(backend: &TestBackend, session: &Session, query: &str) -> TranscriptPage {
    let read = backend
        .get(&format!("/api/conversations/{FIRST}/transcript{query}"), Some(session))
        .await;
    assert_eq!(read.status, StatusCode::OK, "{}", String::from_utf8_lossy(&read.body));
    read.json()
}

// Four requests whose answers are each past half a page: the latest page
// holds the last request alone, and each page before it one more, back to
// the start; the page around a block holds that block's request.
#[tokio::test]
async fn a_long_conversation_reads_in_pages_of_whole_requests() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let long = "a long answer ".repeat(3_000);
    for turn in 1..=4 {
        vendor.respond(answer(&[&format!("{turn}: "), &long], 10, 10));
        socket.chat(&format!("m{turn}"), &format!("Question {turn}")).await;
    }
    // Every block, as the pages and the block reads give them back.
    let live = transcript(&backend, &master, FIRST).await.blocks;

    let latest = page(&backend, &master, "").await;
    assert_eq!(latest.length as usize, live.len());
    assert_eq!(kinds(&latest.blocks), ["user", "text", "response"]);
    assert_eq!(latest.start as usize + latest.blocks.len(), live.len());

    // Back to the start, each page ends where the one after it starts.
    let mut start = latest.start;
    let mut edge = latest.blocks[0].id().clone();
    let mut pages = 1;
    while start > 0 {
        let before = page(&backend, &master, &format!("?before={start}&edge={edge}")).await;
        assert_eq!(before.start as usize + before.blocks.len(), start as usize);
        assert_eq!(before.blocks[0].id(), live[before.start as usize].id());
        start = before.start;
        edge = before.blocks[0].id().clone();
        pages += 1;
    }
    assert_eq!(pages, 4);

    // Around the second answer: its request, which starts with the second message.
    let second = live
        .iter()
        .filter(|block| matches!(block, Block::Text(_)))
        .nth(1)
        .unwrap()
        .id()
        .clone();
    let around = page(&backend, &master, &format!("?around={second}")).await;
    assert!(around.blocks.iter().any(|block| block.id() == &second));
    assert!(matches!(&around.blocks[0], Block::User(_)));

    // An edge that is no longer at its index: the transcript changed under the page.
    let moved = backend
        .get(
            &format!("/api/conversations/{FIRST}/transcript?before={}&edge={second}", latest.start),
            Some(&master),
        )
        .await;
    assert_eq!(moved.refusal(), (StatusCode::CONFLICT, ErrorCode::TranscriptChanged));
    let missing = backend
        .get(&format!("/api/conversations/{FIRST}/transcript?around=no-such-block"), Some(&master))
        .await;
    assert_eq!(missing.refusal(), (StatusCode::NOT_FOUND, ErrorCode::BlockNotFound));
    let both = backend
        .get(
            &format!("/api/conversations/{FIRST}/transcript?before=1&edge={second}&around={second}"),
            Some(&master),
        )
        .await;
    assert_eq!(both.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery));
    backend.close().await;
}

// A call's row needs its title and outcome, not its input or output: the
// page leaves them out, and the block read whole has them.
#[tokio::test]
async fn a_page_leaves_out_what_only_an_open_row_shows_and_a_block_reads_whole() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let input = json!({ "description": "Look something up", "query": "x".repeat(2_000) });
    vendor.respond(tool_use("toolu_1", "no_such_tool", &input));
    vendor.respond(answer(&["Done."], 10, 2));
    socket.chat("m1", "Look it up").await;

    let latest = page(&backend, &master, "").await;
    let Some(Block::ToolCall(light)) = latest.blocks.iter().find(|block| matches!(block, Block::ToolCall(_))) else {
        panic!("{:?}", latest.blocks)
    };
    assert_eq!(serde_json::from_str::<serde_json::Value>(&light.input).unwrap(), json!({ "description": "Look something up" }));
    assert!(light.output.is_empty(), "{:?}", light.output);

    let read = backend
        .get(
            &format!("/api/conversations/{FIRST}/transcript/blocks/{}", light.id),
            Some(&master),
        )
        .await;
    assert_eq!(read.status, StatusCode::OK);
    let WholeBlock { block: Block::ToolCall(whole), .. } = read.json() else {
        panic!("the block is a call")
    };
    assert_eq!(serde_json::from_str::<serde_json::Value>(&whole.input).unwrap(), input);
    assert!(!whole.output.is_empty());
    backend.close().await;
}
