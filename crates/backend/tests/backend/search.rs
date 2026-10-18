//! Finding a conversation (`conversations.md` § Finding a conversation,
//! `web-api-conversations.md` § Search, `storage.md` § Search index): a query lists the
//! conversations whose title matches first, then those whose messages do,
//! each group most recently active first, archived ones marked; a match is
//! the newest message that holds every word, steers among them, with the
//! line around it and the words' places in UTF-16, and the words are marked
//! in a matching title too. Thinking is not searched. An edit that
//! removes a message removes it from the results; the index applies each
//! save on top of the one before, reading a conversation whole only when it
//! missed one; an index of another schema is built again at start, and one
//! that no longer opens is filled again at once, past a conversation that
//! cannot be read. No test calls a real model.

use std::time::Duration;

use demi_agent_server::testing::client_text;
use demi_conversation_socket_protocol::{ClientFrame, ServerFrame};
use demi_provider_common::testing::MockResponse;
use demi_shared_types::BlockId;
use demi_web_api_protocol::error::{ErrorBody, ErrorCode};
use demi_web_api_protocol::search::{SearchResult, SearchResults};
use jiff::SignedDuration;
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{
    FIRST, SECOND, Socket, THIRD, anthropic, answer, choose, create, message, send, text_block,
    thinking_block,
};
use crate::editing::{edit, edit_request, idle};
use crate::support::{Harness, MASTER_EMAIL, MASTER_PASSWORD, Session, TestBackend, eventually};

/// A result as `search` gives it: the conversation, whether it is archived,
/// the marked pieces of its title, and its match's line with the marked
/// pieces of it.
type Found = (String, bool, Vec<String>, Option<(String, Vec<String>)>);

/// The pieces of `text` that UTF-16 `ranges` name.
fn pieces(text: &str, ranges: &[[u32; 2]]) -> Vec<String> {
    let units: Vec<u16> = text.encode_utf16().collect();
    ranges
        .iter()
        .map(|[start, end]| String::from_utf16(&units[*start as usize..*end as usize]).unwrap())
        .collect()
}

/// What a search for `query` answers.
async fn search(backend: &TestBackend, session: &Session, query: &str) -> Vec<Found> {
    let path = format!("/api/search?q={}", urlencode(query));
    let answer = backend.get(&path, Some(session)).await;
    assert_eq!(answer.status, StatusCode::OK, "{}", String::from_utf8_lossy(&answer.body));
    answer
        .json::<SearchResults>()
        .results
        .into_iter()
        .map(|SearchResult { conversation_id, title, title_ranges, archived, r#match, .. }| {
            let r#match = r#match.map(|found| {
                let marked = pieces(&found.text, &found.ranges);
                (found.text, marked)
            });
            (conversation_id.into_string(), archived, pieces(title.as_deref().unwrap_or_default(), &title_ranges), r#match)
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

/// Marked pieces as `search` gives them.
fn marks(pieces: &[&str]) -> Vec<String> {
    pieces.iter().map(|&piece| piece.to_owned()).collect()
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
    // A steer is a message of the user's too: Stop writes it while the
    // request it waited behind still streams.
    vendor.respond(MockResponse::event_stream(": thinking\n\n").stay_open());
    third.send(&send("m2", "Hold on")).await;
    vendor.received(3).await;
    third
        .send(&ClientFrame::Steer {
            comments: Vec::new(),
            steer_id: BlockId::try_from("s1").unwrap(),
            content: client_text("Also check the kubeconfig"),
        })
        .await;
    third
        .until(|frame| matches!(frame, ServerFrame::SteerResult { .. }))
        .await;
    third.stop().await;
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
        (SECOND.to_owned(), false, marks(&["TS2307"]), None),
        (THIRD.to_owned(), true, marks(&[]), found("Clear the ts2307 cache", &["ts2307"])),
        (FIRST.to_owned(), false, marks(&[]), found("Missing 路径 mapping for TS2307", &["TS2307"])),
    ];
    eventually("the index holds every change", || async {
        search(&backend, &master, "ts2307").await == expected
    })
    .await;
    // A word inside a longer one; a two-character Chinese word; every word
    // in one message; thinking is not searched. The user's message may reach
    // the index after the answer the wait above saw, under load.
    let relogin = vec![(THIRD.to_owned(), true, marks(&["login"]), found("relogin is broken", &["login"]))];
    eventually("the index holds the user's message", || async {
        search(&backend, &master, "LOGIN").await == relogin
    })
    .await;
    assert_eq!(
        search(&backend, &master, "路径").await,
        vec![(FIRST.to_owned(), false, marks(&[]), found("Missing 路径 mapping for TS2307", &["路径"]))]
    );
    assert_eq!(
        search(&backend, &master, "mapping ts2307").await,
        vec![(FIRST.to_owned(), false, marks(&[]), found("Missing 路径 mapping for TS2307", &["mapping", "TS2307"]))]
    );
    // The steer Stop wrote may reach the index after the rest, under load.
    let steered = vec![(THIRD.to_owned(), true, marks(&[]), found("Also check the kubeconfig", &["kubeconfig"]))];
    eventually("the index holds the steer", || async {
        search(&backend, &master, "kubeconfig").await == steered
    })
    .await;
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
        (SECOND.to_owned(), false, marks(&["TS2307"]), None),
        (THIRD.to_owned(), true, marks(&[]), found("Clear the ts2307 cache", &["ts2307"])),
    ];
    eventually("the edit is indexed", || async {
        search(&backend, &master, "ts2307").await == edited
    })
    .await;
    assert_eq!(search(&backend, &master, "路径").await, vec![]);
    assert_eq!(
        search(&backend, &master, "fail").await,
        vec![(FIRST.to_owned(), false, marks(&["fail"]), found("Why does it fail?", &["fail"]))]
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

/// The index file of `session`'s user, which tests open beside the backend.
fn index_of(harness: &Harness, session: &Session) -> rusqlite::Connection {
    let path = harness
        .data_dir()
        .join("search")
        .join(format!("{}.sqlite", session.user.id.as_str()));
    rusqlite::Connection::open(path).unwrap()
}

// About a second: four turns and an edit; the index follows each change
// within the 20 ms interval the test sets.
#[tokio::test]
async fn the_index_applies_each_save_on_top_of_the_one_before_and_reads_a_conversation_whole_when_it_missed_one() {
    let vendor = demi_provider_common::testing::MockVendor::start().await;
    let mut harness = Harness::new();
    harness.conversations.search_interval = Duration::from_millis(20);
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut first = Socket::connect(&backend, &master, FIRST).await;
    first.open().await;
    vendor.respond(answer(&["First answer"], 1, 1));
    first.chat("m1", "Where is alphaword kept?").await;
    let alpha = vec![(FIRST.to_owned(), false, marks(&["alphaword"]), found("Where is alphaword kept?", &["alphaword"]))];
    let first_answer = vec![(FIRST.to_owned(), false, marks(&[]), found("First answer", &["First"]))];
    eventually("the first turn is indexed", || async {
        search(&backend, &master, "first").await == first_answer
    })
    .await;

    // A turn writes its two rows from the saves alone: nothing is read
    // whole, and no row of the turn before is touched.
    let start = backend.search_work();
    vendor.respond(answer(&["Second answer bravoword"], 1, 1));
    first.chat("m2", "And betaword?").await;
    let bravo = vec![(FIRST.to_owned(), false, marks(&[]), found("Second answer bravoword", &["bravoword"]))];
    eventually("the second turn is indexed", || async {
        search(&backend, &master, "bravoword").await == bravo
    })
    .await;
    let work = backend.search_work();
    assert_eq!((work.whole - start.whole, work.rows - start.rows), (0, 2));

    // The index misses the last save, as after a crash between its commit
    // and its apply: the next save does not apply on top of it, and the
    // conversation is read whole, which brings the missed row back.
    let index = index_of(&harness, &master);
    index
        .execute("DELETE FROM texts WHERE text = 'Second answer bravoword'", [])
        .unwrap();
    index
        .execute("UPDATE agents SET revision = revision - 1 WHERE agent = 0", [])
        .unwrap();
    drop(index);
    assert_eq!(search(&backend, &master, "bravoword").await, vec![]);
    let start = backend.search_work();
    vendor.respond(answer(&["Third answer charlieword"], 1, 1));
    first.chat("m3", "What about gammaword?").await;
    let charlie = vec![(FIRST.to_owned(), false, marks(&[]), found("Third answer charlieword", &["charlieword"]))];
    eventually("the third turn is indexed", || async {
        search(&backend, &master, "charlieword").await == charlie
    })
    .await;
    eventually("the missed save is read back", || async {
        search(&backend, &master, "bravoword").await == bravo
    })
    .await;
    assert_eq!(backend.search_work().whole - start.whole, 1);

    // An edit of the second message cuts it and everything after it: their
    // rows go, from the save alone, and the rows before the cut stay.
    let start = backend.search_work();
    let request = edit_request(&mut first, "And betaword?", "edit-1", "And deltaword?").await;
    vendor.respond(answer(&["Edited answer echoword"], 1, 1));
    edit(&mut first, &ClientFrame::EditAndSend { request }).await;
    first.until(idle).await;
    drop(first);
    let echo = vec![(FIRST.to_owned(), false, marks(&[]), found("Edited answer echoword", &["echoword"]))];
    eventually("the edit is indexed", || async {
        search(&backend, &master, "echoword").await == echo
    })
    .await;
    for cut in ["betaword", "bravoword", "gammaword", "charlieword"] {
        assert_eq!(search(&backend, &master, cut).await, vec![], "{cut}");
    }
    assert_eq!(search(&backend, &master, "alphaword").await, alpha);
    assert_eq!(backend.search_work().whole - start.whole, 0);
    backend.close().await;
}

// About a second: two turns and a reset of the index.
#[tokio::test]
async fn a_reset_index_is_filled_again_at_once_past_a_conversation_that_cannot_be_read() {
    let vendor = demi_provider_common::testing::MockVendor::start().await;
    let mut harness = Harness::new();
    harness.conversations.search_interval = Duration::from_millis(20);
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    for id in [FIRST, SECOND] {
        create(&backend, &master, id).await;
        choose(&backend, &master, id, &provider, "claude-opus-4-8").await;
    }
    let mut second = Socket::connect(&backend, &master, SECOND).await;
    second.open().await;
    vendor.respond(answer(&["Kept in the foxtrot folder"], 1, 1));
    second.chat("m1", "Where is it?").await;
    drop(second);
    // The first conversation is the most recently active, so it is read
    // first.
    harness.clock.advance(SignedDuration::from_mins(1));
    let mut first = Socket::connect(&backend, &master, FIRST).await;
    first.open().await;
    vendor.respond(answer(&["Nothing here"], 1, 1));
    first.chat("m1", "Anything?").await;
    drop(first);
    let foxtrot = vec![(SECOND.to_owned(), false, marks(&[]), found("Kept in the foxtrot folder", &["foxtrot"]))];
    eventually("both conversations are indexed", || async {
        search(&backend, &master, "foxtrot").await == foxtrot
            && search(&backend, &master, "nothing here").await.len() == 1
    })
    .await;

    // The first conversation's root no longer reads, and the index no
    // longer opens: it is built again empty, and filled again at once, the
    // idle second conversation too, past the first.
    let database = harness.data_dir().join("conversations").join(format!("{FIRST}.sqlite"));
    rusqlite::Connection::open(database)
        .unwrap()
        .execute(
            r#"UPDATE blocks SET block = '{"type":"not a block"}'
               WHERE idx = 0 AND node_id = (SELECT id FROM nodes WHERE parent_id IS NULL)"#,
            [],
        )
        .unwrap();
    backend.corrupt_search_index(&master.user.id).await;
    eventually("the reset index is filled again", || async {
        search(&backend, &master, "foxtrot").await == foxtrot
    })
    .await;
    backend.close().await;
}
