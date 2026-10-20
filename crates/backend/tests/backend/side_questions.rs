//! Side questions on the backend.
//!
//! A side question is asked about a reply's words over HTTP and answered in a
//! side conversation:
//!
//! - The page reads its transcript by its node.
//! - It is kept with its anchor and its fold across a restart.
//! - It is announced to the pages through the summary's revision.
//! - It is counted under the conversation in the usage ledger.
//! - A Fork never copies it, and a share never shows it.
//!
//! No test calls a real model.

use demi_provider_common::testing::{MockResponse, MockVendor, sse_body};
use demi_shared_types::Block;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::shares::ShareAnswer;
use demi_conversation_socket_protocol::{AgentRole, SideQuestion};
use demi_web_api_protocol::side_questions::SideQuestionAnswer;
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::conversations::{
    FIRST, SECOND, Socket, agent_history, answer, anthropic, choose, configured, create, detail, entry, last_text,
};
use crate::support::{Session, TestBackend, eventually, Harness};
use crate::uploads::{png, upload};
use base64::{Engine, prelude::BASE64_STANDARD};

const SIDE: &str = "5b0e7c1a-2f3d-4e6a-9b8c-7d6e5f4a3b2c";

fn asking(block: &str, question: &str) -> Value {
    json!({ "id": SIDE, "blockId": block, "start": 4, "end": 15, "quote": "stale cache", "question": question })
}

/// The conversation's side questions, as its agents list them.
async fn listed(backend: &TestBackend, session: &Session, conversation: &str) -> Vec<SideQuestion> {
    detail(backend, session, conversation)
        .await
        .entries
        .into_iter()
        .filter_map(|entry| match entry.role {
            AgentRole::Side(side) => Some(side),
            _ => None,
        })
        .collect()
}

/// The revision of the conversation's agents, which each change of a side
/// question raises.
async fn revision(backend: &TestBackend, session: &Session) -> u64 {
    detail(backend, session, FIRST).await.agents.revision
}

// The conversation's agent answers; the user asks about its words.
//
// - The side's request ends with the asked words, the instruction and the
//   question. Its answer is its transcript's, read by its node, and the
//   conversation's own transcript gains nothing.
// - The same request again finds it; another question under its id is
//   refused.
// - The fold is kept, each change raises the agents' revision, and both
//   survive a restart.
// - Its request is counted under the conversation, under no agent.
// - A Fork of the reply copies no side question and a share reaches none of
//   its blocks.
//
// A backend with a mock vendor and a restart: about two seconds.
#[tokio::test]
async fn a_side_question_is_answered_beside_the_conversation_and_kept_with_its_reply() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(answer(&["The stale cache keeps old totals."], 1, 1));
    socket.chat("m1", "Why is the card wrong?").await;
    let history = socket.live().await;
    let reply = history
        .iter()
        .rev()
        .find(|block| matches!(block, Block::Text(_)))
        .expect("the agent replied")
        .id()
        .to_string();
    let before = revision(&backend, &master).await;

    vendor.respond(answer(&["Its TTL is an hour."], 1, 1));
    let path = format!("/api/conversations/{FIRST}/side-questions");
    let asked = backend.post(&path, Some(&master), asking(&reply, "Why stale?")).await;
    assert_eq!(asked.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&asked.body));
    let created = asked.json::<SideQuestionAnswer>().side_question;
    assert_eq!(
        (created.block_id.as_str(), created.start, created.end, created.quote.as_str(), created.question.as_str()),
        (reply.as_str(), 4, 15, "stale cache", "Why stale?")
    );
    assert_eq!((created.title, created.folded), (None, false));
    eventually("the side question answers", || async {
        last_text(&agent_history(&backend, &master, FIRST, Some(SIDE)).await.0) == "Its TTL is an hour."
    })
    .await;
    let requests = vendor.requests();
    let side_body = String::from_utf8_lossy(&requests[1].body).into_owned();
    assert!(side_body.contains("Why is the card wrong?"), "{side_body}");
    assert!(side_body.contains("The user asks this aside from the conversation"), "{side_body}");
    assert!(side_body.contains("Why stale?"), "{side_body}");
    assert_eq!(socket.live().await, history, "the conversation's transcript gains nothing");
    assert!(revision(&backend, &master).await > before);

    let again = backend.post(&path, Some(&master), asking(&reply, "Why stale?")).await;
    assert_eq!(again.status, StatusCode::OK);
    let taken = backend.post(&path, Some(&master), asking(&reply, "Something else?")).await;
    assert_eq!(taken.refusal(), (StatusCode::CONFLICT, ErrorCode::SideQuestionConflict));
    let elsewhere = backend
        .post(&path, Some(&master), json!({ "id": "another-side", "blockId": "no-such-block", "start": 0, "end": 1, "quote": "x", "question": "Why?" }))
        .await;
    assert_eq!(elsewhere.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidSideQuestionTarget));

    let unfolded = revision(&backend, &master).await;
    let folded = backend
        .patch(&format!("{path}/{SIDE}"), &master, json!({ "folded": true }))
        .await
        .json::<SideQuestionAnswer>()
        .side_question;
    assert!(folded.folded);
    assert!(revision(&backend, &master).await > unfolded);

    // Counted under the conversation, the side's under no agent.
    let ledger: Vec<(Option<String>, Option<i64>)> = {
        let database = harness.control_database();
        let mut statement = database
            .prepare("SELECT conversation_id, agent FROM usage_ledger WHERE purpose = 'turn' ORDER BY rowid")
            .unwrap();
        statement
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap()
    };
    assert_eq!(ledger, [(Some(FIRST.to_owned()), Some(0)), (Some(FIRST.to_owned()), None)]);

    // A Fork of the reply copies none.
    let forked = backend
        .post(&format!("/api/conversations/{FIRST}/fork"), Some(&master), json!({ "id": SECOND, "blockId": reply }))
        .await;
    assert_eq!(forked.status, StatusCode::CREATED);
    assert!(listed(&backend, &master, SECOND).await.is_empty());

    // A share reaches none of its blocks.
    let side_answer = agent_history(&backend, &master, FIRST, Some(SIDE))
        .await
        .0
        .into_iter()
        .rev()
        .find(|block| matches!(block, Block::Text(_)))
        .unwrap();
    let share = backend
        .post(&format!("/api/conversations/{FIRST}/shares"), Some(&master), json!({ "kind": "conversation" }))
        .await
        .json::<ShareAnswer>();
    let ShareAnswer::Conversation { token, .. } = share else {
        panic!("the conversation's share: {share:?}");
    };
    let reached = backend
        .get(&format!("/api/shares/{token}/transcript/blocks/{}", side_answer.id()), None)
        .await;
    assert_eq!(reached.status, StatusCode::NOT_FOUND);
    let shared = backend.get(&format!("/api/shares/{token}/transcript"), None).await;
    assert!(!String::from_utf8_lossy(&shared.body).contains("Its TTL is an hour."));

    // Kept across a restart, with its anchor and its fold.
    drop(socket);
    let address = backend.address();
    backend.close().await;
    let backend = harness.start_at(address).await;
    let kept = listed(&backend, &master, FIRST).await;
    assert_eq!(kept, [folded]);
}

// The user asks about a picture of the reply: the side's request carries the
// picture before the question, and the listed side question keeps it with its
// place among the block's pictures. A backend with a mock vendor: under a
// second.
#[tokio::test]
async fn a_side_question_about_a_picture_sends_the_picture_and_keeps_it() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(answer(&["Here is the chart."], 1, 1));
    socket.chat("m1", "Show me the chart.").await;
    let reply = socket
        .live()
        .await
        .iter()
        .rev()
        .find(|block| matches!(block, Block::Text(_)))
        .expect("the agent replied")
        .id()
        .to_string();

    vendor.respond(answer(&["The 3G one."], 1, 1));
    // The picture is a blob of the user's, as the page keeps one.
    let chart = png(1);
    let kept = upload(&backend, &master, "chart.png", "image/png", &chart).await;
    let picture = json!({ "type": "ref", "ref": kept.sha256, "mediaType": "image/png" });
    let asked = backend
        .post(
            &format!("/api/conversations/{FIRST}/side-questions"),
            Some(&master),
            json!({ "id": SIDE, "blockId": reply, "start": 0, "end": 1, "quote": "The chart", "picture": picture, "question": "Which connection?" }),
        )
        .await;
    assert_eq!(asked.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&asked.body));
    eventually("the side question answers", || async {
        last_text(&agent_history(&backend, &master, FIRST, Some(SIDE)).await.0) == "The 3G one."
    })
    .await;
    let side_body = body(&vendor.requests()[1]);
    let last = side_body["messages"].as_array().unwrap().last().unwrap().clone();
    assert_eq!(last["content"][0]["type"], "image", "{last}");
    assert_eq!(last["content"][0]["source"]["data"], BASE64_STANDARD.encode(&chart), "{last}");
    assert!(last["content"][1]["text"].as_str().unwrap().ends_with("Which connection?"), "{last}");
    let kept = listed(&backend, &master, FIRST).await;
    assert_eq!(serde_json::to_value(&kept[0].picture).unwrap(), picture);
    backend.close().await;
}

/// A Responses stream that answers `text`.
fn responses_answer(text: &str) -> MockResponse {
    MockResponse::event_stream(sse_body(&[
        json!({ "type": "response.output_text.delta", "delta": text }),
        json!({ "type": "response.completed", "response": { "usage": { "input_tokens": 3, "output_tokens": 1 } } }),
    ]))
}

/// The JSON body of a request the vendor received.
fn body(request: &demi_provider_common::testing::RecordedRequest) -> Value {
    serde_json::from_slice(&request.body).expect("a request body is JSON")
}

/// The conversation answers once, the user asks about the answer's words,
/// then sends the conversation its next message; answers the vendor's
/// three requests, the side question's first and the conversation's next.
async fn side_and_next(backend: &TestBackend, master: &Session, vendor: &MockVendor, provider: &str, model: &str, answers: [MockResponse; 3]) -> (Value, Value) {
    let [reply, side, next] = answers;
    create(backend, master, FIRST).await;
    choose(backend, master, FIRST, provider, model).await;
    let mut socket = Socket::connect(backend, master, FIRST).await;
    socket.open().await;
    vendor.respond(reply);
    socket.chat("m1", "Why is the card wrong?").await;
    let reply = socket
        .live()
        .await
        .iter()
        .rev()
        .find(|block| matches!(block, Block::Text(_)))
        .expect("the agent replied")
        .id()
        .to_string();
    vendor.respond(side);
    let path = format!("/api/conversations/{FIRST}/side-questions");
    let asked = backend.post(&path, Some(master), asking(&reply, "Why stale?")).await;
    assert_eq!(asked.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&asked.body));
    eventually("the side question answers", || async { vendor.requests().len() == 2 }).await;
    eventually("the side question closes", || async {
        !agent_history(backend, master, FIRST, Some(SIDE)).await.0.is_empty()
    })
    .await;
    vendor.respond(next);
    socket.chat("m2", "Fix it.").await;
    let requests = vendor.requests();
    assert_eq!(requests.len(), 3);
    (body(&requests[1]), body(&requests[2]))
}

// On an OpenAI Responses entry, the side question's first request carries
// the conversation's prompt cache key, its instructions and tools, and the
// conversation's next input up to the conversation's own message; then its
// own message. The conversation's next then hears what was asked aside. A backend with a mock vendor: about a second.
#[tokio::test]
async fn a_side_questions_first_openai_request_is_the_conversations_next_with_its_cache_key() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = entry(
        &backend,
        &master,
        json!({
            "source": "compatible", "providerType": "openai", "wireApi": "responses", "label": "Proxy",
            "apiKey": "sk-test", "baseUrl": vendor.url("/v1"), "models": [configured(1000)]
        }),
    )
    .await;
    let (side, next) = side_and_next(
        &backend,
        &master,
        &vendor,
        &provider,
        "m",
        [responses_answer("The stale cache keeps old totals."), responses_answer("Its TTL is an hour."), responses_answer("Fixed.")],
    )
    .await;
    assert_eq!(side["prompt_cache_key"], next["prompt_cache_key"]);
    assert!(side["prompt_cache_key"].is_string(), "{side}");
    assert_eq!(side["instructions"], next["instructions"]);
    assert_eq!(side["tools"], next["tools"]);
    let (side_input, next_input) = (side["input"].as_array().unwrap(), next["input"].as_array().unwrap());
    // What the side shares ends before the conversation's own message,
    // after which the conversation hears what was asked aside.
    let shared = next_input
        .iter()
        .position(|item| item.to_string().contains("Fix it."))
        .expect("the conversation's next carries its message");
    assert_eq!(side_input.len(), shared + 1);
    assert_eq!(side_input[..shared], next_input[..shared]);
    assert!(side_input[shared].to_string().contains("Why stale?"), "{side}");
    let told = serde_json::to_string(&next_input[shared + 1..]).unwrap();
    assert!(told.contains("Q: Why stale?\\nA: Its TTL is an hour."), "{told}");
}

// On an Anthropic entry, the side question's first request marks the
// conversation's history for the cache where the conversation's next one
// does, the reply's response included, so the two share their prefix byte
// for byte. A backend with a mock vendor: about a second.
#[tokio::test]
async fn a_side_questions_first_anthropic_request_shares_the_conversations_cached_prefix() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    let (side, next) = side_and_next(
        &backend,
        &master,
        &vendor,
        &provider,
        "claude-opus-4-8",
        [answer(&["The stale cache keeps old totals."], 1, 1), answer(&["Its TTL is an hour."], 1, 1), answer(&["Fixed."], 1, 1)],
    )
    .await;
    assert_eq!(side["system"], next["system"]);
    assert_eq!(side["tools"], next["tools"]);
    let (side_messages, next_messages) = (side["messages"].as_array().unwrap(), next["messages"].as_array().unwrap());
    assert_eq!(side_messages.len(), next_messages.len());
    let shared = next_messages.len() - 1;
    assert_eq!(side_messages[..shared], next_messages[..shared]);
    assert!(side_messages[shared].to_string().contains("Why stale?"), "{side}");
}
