//! A conversation's history read a page at a time.
//!
//! - The page of a conversation, with no byte limit.
//! - A page around a block.
//! - The refusal of a page whose edge moved.
//! - The light form and a block read whole.

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
        .get(&format!("/api/conversations/{FIRST}/agents/{FIRST}/transcript{query}"), Some(session))
        .await;
    assert_eq!(read.status, StatusCode::OK, "{}", String::from_utf8_lossy(&read.body));
    read.json()
}

// Four requests whose answers are each far past the old 64 KB limit of a
// page: the conversation is one page, the whole of it, since a page is a
// thousand blocks with no byte limit
// (`crates/conversation-socket-protocol/src/transcript.rs`); the page around a
// block holds that block, and a page before its first block is empty.
#[tokio::test]
async fn a_long_answer_does_not_split_a_page() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let long = "a long answer ".repeat(10_000);
    for turn in 1..=4 {
        vendor.respond(answer(&[&format!("{turn}: "), &long], 10, 10));
        socket.chat(&format!("m{turn}"), &format!("Question {turn}")).await;
    }
    // Every block, as the pages and the block reads give them back.
    let live = transcript(&backend, &master, FIRST).await.blocks;

    let latest = page(&backend, &master, "").await;
    assert_eq!(latest.length as usize, live.len());
    assert_eq!(latest.start, 0);
    assert_eq!(latest.blocks.len(), live.len());
    assert_eq!(kinds(&latest.blocks), kinds(&live));

    // Nothing lies before the first block.
    let first = latest.blocks[0].id().clone();
    let before = page(&backend, &master, &format!("?before=0&edge={first}")).await;
    assert!(before.blocks.is_empty());

    // Around the second answer: a page that holds it.
    let second = live
        .iter()
        .filter(|block| matches!(block, Block::Text(_)))
        .nth(1)
        .unwrap()
        .id()
        .clone();
    let around = page(&backend, &master, &format!("?around={second}")).await;
    assert!(around.blocks.iter().any(|block| block.id() == &second));

    // An edge that is no longer at its index: the transcript changed under the page.
    let moved = backend
        .get(
            &format!("/api/conversations/{FIRST}/agents/{FIRST}/transcript?before={}&edge={second}", latest.start),
            Some(&master),
        )
        .await;
    assert_eq!(moved.refusal(), (StatusCode::CONFLICT, ErrorCode::TranscriptChanged));
    let missing = backend
        .get(&format!("/api/conversations/{FIRST}/agents/{FIRST}/transcript?around=no-such-block"), Some(&master))
        .await;
    assert_eq!(missing.refusal(), (StatusCode::NOT_FOUND, ErrorCode::BlockNotFound));
    let both = backend
        .get(
            &format!("/api/conversations/{FIRST}/agents/{FIRST}/transcript?before=1&edge={second}&around={second}"),
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
            &format!("/api/conversations/{FIRST}/agents/{FIRST}/blocks/{}", light.id),
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
