//! Finding a conversation (`product.md` § Finding a conversation,
//! `web-api.md` § Search, `storage.md` § Search index): a query lists the
//! conversations whose title matches first, then those whose messages do,
//! each group most recently active first, archived ones marked; a match is
//! the newest message that holds every word, with the line around it and
//! the words' places in UTF-16. Thinking is not searched. An edit that
//! removes a message removes it from the results, and an index of another
//! schema is built again at start. No test calls a real model.

use std::time::Duration;

use demi_conversation_socket_protocol::ClientFrame;
use demi_web_api_protocol::error::{ErrorBody, ErrorCode};
use demi_web_api_protocol::search::{SearchResult, SearchResults};
use jiff::SignedDuration;
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{
    FIRST, SECOND, Socket, THIRD, anthropic, answer, choose, create, message, text_block,
    thinking_block,
};
use crate::editing::{edit, edit_request, idle};
use crate::support::{Harness, MASTER_EMAIL, MASTER_PASSWORD, Session, TestBackend, eventually};

/// What a search for `query` answers, each result as the conversation, whether
/// it is archived, and its match's line with the marked pieces of it.
async fn search(
    backend: &TestBackend,
    session: &Session,
    query: &str,
) -> Vec<(String, bool, Option<(String, Vec<String>)>)> {
    let path = format!("/api/search?q={}", urlencode(query));
    let answer = backend.get(&path, Some(session)).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", String::from_utf8_lossy(&answer.body));
    answer
        .json::<SearchResults>()
        .results
        .into_iter()
        .map(|SearchResult { conversation_id, archived, r#match, .. }| {
            let r#match = r#match.map(|found| {
                let units: Vec<u16> = found.text.encode_utf16().collect();
                let marked = found
                    .ranges
                    .iter()
                    .map(|[start, end]| String::from_utf16(&units[*start as usize..*end as usize]).unwrap())
                    .collect();
                (found.text, marked)
            });
            (conversation_id.into_string(), archived, r#match)
        })
        .collect()
}

/// `text` as a query parameter's value.
fn urlencode(text: &str) -> String {
    text.bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                char::from(byte).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

/// A match as `search` gives it.
fn found(line: &str, marked: &[&str]) -> Option<(String, Vec<String>)> {
    Some((line.to_owned(), marked.iter().map(|&piece| piece.to_owned()).collect()))
}

// About a second: three turns, an edit and a restart; the index follows
// each change within the 20 ms interval the test sets.
#[tokio::test]
async fn a_search_lists_title_matches_then_the_newest_messages_and_follows_edits_and_a_rebuilt_index() {
    let vendor = demi_provider_common::testing::MockVendor::start().await;
    let mut harness = Harness::new();
    harness.conversations.search_interval = Duration::from_millis(20);
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    for id in [FIRST, SECOND, THIRD] {
        create(&backend, &master, id).await;
        choose(&backend, &master, id, &provider, "claude-opus-4-8").await;
    }
    let mut first = Socket::connect(&backend, &master, FIRST).await;
    first.open().await;
    vendor.respond(message(
        vec![
            thinking_block(0, "unsearchedthought", "sig"),
            text_block(1, &["Missing 路径 mapping", " for TS2307"]),
        ],
        "end_turn",
        json!({ "input_tokens": 1, "output_tokens": 0 }),
        1,
    ));
    first.chat("m1", "Why does the build fail?").await;
    // The third conversation's message is written a minute later, so it is
    // the most recently active.
    harness.clock.advance(SignedDuration::from_mins(1));
    let mut third = Socket::connect(&backend, &master, THIRD).await;
    third.open().await;
    vendor.respond(answer(&["Clear the ts2307 cache"], 1, 1));
    third.chat("m1", "relogin is broken").await;
    drop(third);
    let renamed = backend
        .patch(&format!("/api/conversations/{SECOND}"), &master, json!({ "title": "TS2307 after the split" }))
        .await;
    assert_eq!(renamed.status, StatusCode::OK);
    let archived = backend
        .patch(&format!("/api/conversations/{THIRD}"), &master, json!({ "archived": true }))
        .await;
    assert_eq!(archived.status, StatusCode::OK);

    let expected = vec![
        (SECOND.to_owned(), false, None),
        (THIRD.to_owned(), true, found("Clear the ts2307 cache", &["ts2307"])),
        (FIRST.to_owned(), false, found("Missing 路径 mapping for TS2307", &["TS2307"])),
    ];
    eventually("the index holds every change", || async {
        search(&backend, &master, "ts2307").await == expected
    })
    .await;
    // A word inside a longer one; a two-character Chinese word; every word
    // in one message; thinking is not searched.
    assert_eq!(
        search(&backend, &master, "LOGIN").await,
        vec![(THIRD.to_owned(), true, found("relogin is broken", &["login"]))]
    );
    assert_eq!(
        search(&backend, &master, "路径").await,
        vec![(FIRST.to_owned(), false, found("Missing 路径 mapping for TS2307", &["路径"]))]
    );
    assert_eq!(
        search(&backend, &master, "mapping ts2307").await,
        vec![(FIRST.to_owned(), false, found("Missing 路径 mapping for TS2307", &["mapping", "TS2307"]))]
    );
    assert_eq!(search(&backend, &master, "relogin cache").await, vec![]);
    assert_eq!(search(&backend, &master, "unsearchedthought").await, vec![]);
    let empty = backend.get("/api/search?q=%20%20", Some(&master)).await;
    assert_eq!(empty.status, StatusCode::BAD_REQUEST);
    assert_eq!(empty.json::<ErrorBody>().code, ErrorCode::InvalidQuery);

    // An edit of the first message replaces it and removes its answer; the
    // title the message gave stays.
    let request = edit_request(&mut first, "Why does the build fail?", "edit-1", "Why does it fail?").await;
    vendor.respond(answer(&["Fixed now"], 1, 1));
    edit(&mut first, &ClientFrame::EditAndSend { request }).await;
    first.until(idle).await;
    drop(first);
    let edited = vec![
        (SECOND.to_owned(), false, None),
        (THIRD.to_owned(), true, found("Clear the ts2307 cache", &["ts2307"])),
    ];
    eventually("the edit is indexed", || async {
        search(&backend, &master, "ts2307").await == edited
    })
    .await;
    assert_eq!(search(&backend, &master, "路径").await, vec![]);
    assert_eq!(
        search(&backend, &master, "fail").await,
        vec![(FIRST.to_owned(), false, found("Why does it fail?", &["fail"]))]
    );

    // An index of another schema is built again at the next start, in the
    // background, from the conversations.
    backend.close().await;
    let index = harness
        .data_dir()
        .join("search")
        .join(format!("{}.sqlite", master.user.id.as_str()));
    rusqlite::Connection::open(&index)
        .unwrap()
        .pragma_update(None, "user_version", 1)
        .unwrap();
    let backend = harness.start().await;
    let master = backend.login(MASTER_EMAIL, MASTER_PASSWORD).await;
    eventually("the index is built again", || async {
        search(&backend, &master, "ts2307").await == edited
    })
    .await;
    backend.close().await;
}
