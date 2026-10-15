//! Sharing (`sharing.md`, `web-api.md` § Sharing): an answer's share holds
//! the message and the text that ends the reply to it; a conversation's
//! holds every agent's transcript but its hidden inputs, with the media its
//! blocks show, as it was when it was taken until the owner takes it again;
//! the public routes answer the token alone, ignore a session and answer
//! every miss alike; and a share goes with its conversation or its Stop
//! Sharing, at once.

use std::sync::Arc;

use demi_provider_common::testing::MockVendor;
use demi_shared_types::{Block, ToolView};
use demi_web_api_protocol::conversations::{TranscriptPage, WholeBlock};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::shares::{ShareAnswer, ShareKind, SharedConversation};
use demi_web_api_protocol::state::SyncEvent;
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{
    FIRST, Socket, anthropic, answer, choose, create, kinds, message, summary, text_block, tool_use_block,
    transcript,
};
use crate::subagents::{Scripts, say, shell, tree_on};
use crate::support::{Answer, Harness, Session, TestBackend, eventually};
use crate::uploads::{PNG, upload, with_upload};

/// Shares `target` of `FIRST` as `session`.
async fn share(backend: &TestBackend, session: &Session, target: serde_json::Value) -> Answer {
    backend
        .post(&format!("/api/conversations/{FIRST}/shares"), Some(session), target)
        .await
}

fn shared(answer: &Answer) -> ShareAnswer {
    assert_eq!(answer.status, StatusCode::OK, "{}", String::from_utf8_lossy(&answer.body));
    answer.json()
}

/// A public read of `path` under `/api/shares/<token>`, with the cookie of
/// `session` when there is one, which the route ignores.
async fn public(backend: &TestBackend, token: &str, path: &str, session: Option<&Session>) -> Answer {
    let read = backend.get(&format!("/api/shares/{token}{path}"), session).await;
    for (name, value) in [
        ("x-robots-tag", "noindex, nofollow"),
        ("referrer-policy", "no-referrer"),
        ("cache-control", "private, no-store"),
    ] {
        assert_eq!(read.headers.get(name).and_then(|value| value.to_str().ok()), Some(value), "{name} of {path}");
    }
    read
}

async fn page(backend: &TestBackend, token: &str, query: &str) -> TranscriptPage {
    let read = public(backend, token, &format!("/transcript{query}"), None).await;
    assert_eq!(read.status, StatusCode::OK, "{}", String::from_utf8_lossy(&read.body));
    read.json()
}

/// The texts of `blocks`.
fn texts(blocks: &[Block]) -> Vec<String> {
    blocks
        .iter()
        .filter_map(|block| match block {
            Block::Text(text) => Some(text.text.clone()),
            _ => None,
        })
        .collect()
}

/// Asserts `read` is the one answer of every miss of a share.
fn missed(read: &Answer, what: &str) {
    assert_eq!(read.refusal(), (StatusCode::NOT_FOUND, ErrorCode::ShareNotFound), "{what}");
    assert_eq!(read.error().message, "This shared link does not exist", "{what}");
}

// Two questions, the first answered after a call; the first answer is shared.
#[tokio::test]
async fn an_answers_share_holds_its_message_and_answer_alone_until_the_owner_stops_sharing() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let stranger = backend.make_user(&master, "stranger@example.test", "user").await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(message(
        vec![
            text_block(0, &["Looking it up."]),
            tool_use_block(1, "toolu_1", "no_such_tool", &json!({ "description": "Look" })),
        ],
        "tool_use",
        json!({ "input_tokens": 1, "output_tokens": 0 }),
        1,
    ));
    vendor.respond(answer(&["Answer one."], 10, 2));
    socket.chat("m1", "Question one").await;
    vendor.respond(answer(&["Answer two."], 10, 2));
    socket.chat("m2", "Question two").await;
    let blocks = transcript(&backend, &master, FIRST).await.blocks;
    let text = |wanted: &str| {
        blocks
            .iter()
            .find_map(|block| match block {
                Block::Text(text) if text.text == wanted => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no text {wanted}: {blocks:?}"))
    };
    let (looking, first) = (text("Looking it up."), text("Answer one."));
    assert!(looking.forkable, "a text before a call is complete, yet the agent went on");
    let mut sync = backend.sync(&master).await;
    sync.snapshot().await;

    let ShareAnswer::Answer { token, url, block_id, taken_at } =
        shared(&share(&backend, &master, json!({ "kind": "answer", "blockId": first.id })).await)
    else {
        panic!("an answer's share")
    };
    let token = token.as_str().to_owned();
    assert_eq!(block_id, first.id);
    assert_eq!(url, format!("http://{}/s/{token}", backend.address()));
    assert_eq!(token.len(), 22, "128 bits in URL-safe base64");
    // The owner's pages learn of it from the conversation's summary.
    sync.until(|event| {
        matches!(event, SyncEvent::Conversation { conversation }
            if conversation.shares.iter().any(|share| share.token().as_str() == token))
    })
    .await;
    // Sharing it again opens the same link.
    let again = shared(&share(&backend, &master, json!({ "kind": "answer", "blockId": first.id })).await);
    assert!(matches!(again, ShareAnswer::Answer { token: same, .. } if same.as_str() == token));

    // Anyone reads the message and its answer, and nothing else.
    let about: SharedConversation = public(&backend, &token, "", None).await.json();
    assert_eq!(about.kind, ShareKind::Answer);
    assert_eq!(about.taken_at, taken_at);
    let read = page(&backend, &token, "").await;
    assert_eq!(kinds(&read.blocks), ["user", "text"]);
    assert_eq!(texts(&read.blocks), ["Answer one."]);
    let Block::User(question) = &read.blocks[0] else { unreachable!() };
    assert_eq!(serde_json::to_value(&question.content).unwrap()[0]["text"], "Question one");
    // Another user's session changes nothing.
    let as_stranger = public(&backend, &token, "/transcript", Some(&stranger)).await;
    assert_eq!(as_stranger.json::<TranscriptPage>(), read);

    // Only the text that ends a reply is an answer, and an answer is never
    // taken again.
    for refused in [&looking.id, &question.id] {
        let refusal = share(&backend, &master, json!({ "kind": "answer", "blockId": refused })).await;
        assert_eq!(refusal.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::ShareTargetInvalid));
    }
    let retaken = backend
        .put(&format!("/api/conversations/{FIRST}/shares/{token}"), &master, json!({}))
        .await;
    assert_eq!(retaken.refusal(), (StatusCode::CONFLICT, ErrorCode::ShareNotUpdatable));
    // Another user neither shares the conversation nor stops its share.
    let theirs = share(&backend, &stranger, json!({ "kind": "conversation" })).await;
    assert_eq!(theirs.refusal(), (StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound));
    let stopped = backend
        .delete(&format!("/api/conversations/{FIRST}/shares/{token}"), &stranger)
        .await;
    missed(&stopped, "another user's Stop Sharing");

    // Stop Sharing ends every read at once, as a token that never existed.
    let stopped = backend
        .delete(&format!("/api/conversations/{FIRST}/shares/{token}"), &master)
        .await;
    assert_eq!(stopped.status, StatusCode::NO_CONTENT);
    for path in ["", "/transcript", &format!("/transcript/blocks/{}", first.id)] {
        missed(&public(&backend, &token, path, None).await, path);
        missed(&public(&backend, "AAAAAAAAAAAAAAAAAAAAAA", path, None).await, path);
    }
    assert!(summary(&backend, &master, FIRST).await.shares.is_empty());
    backend.close().await;
}

// A few seconds: the root runs two shell jobs on a real device, one of them
// the spawn of a child.
#[tokio::test]
async fn a_conversations_share_shows_every_agent_its_media_and_commands_as_they_were_until_taken_again() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _paired, _root, _provider) = tree_on(&scripts, Harness::new()).await;
    let stranger = backend.make_user(&master, "stranger@example.test", "user").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let shot = upload(&backend, &master, "shot.png", "image/png", &PNG).await;
    let unsent = demi_agent_store::testing::png(5, 5, 1);
    let unsent = upload(&backend, &master, "other.png", "image/png", unsent.as_bytes()).await;
    scripts.child(vec![say("hi from the child")]);
    scripts.root(
        FIRST,
        vec![
            shell("t1", "printf 'hello from the command\\n'"),
            shell("t2", "demi agent spawn --description helper <<< 'Say hi'"),
            say("asked the helper"),
            // The child's completion wakes the idle parent.
            say("the helper said hi"),
        ],
    );
    socket.send(&with_upload("m1", "Look at this", &[(&shot, "shot.png")])).await;
    eventually("the parent hears its child", || async {
        texts(&transcript(&backend, &master, FIRST).await.blocks).contains(&"the helper said hi".to_owned())
    })
    .await;
    socket.until_idle().await;
    let own = transcript(&backend, &master, FIRST).await;
    let child = own.subagents[0].child.number;

    let ShareAnswer::Conversation { token, .. } =
        shared(&share(&backend, &master, json!({ "kind": "conversation" })).await)
    else {
        panic!("a conversation's share")
    };
    let token = token.as_str().to_owned();
    let about: SharedConversation = public(&backend, &token, "", Some(&stranger)).await.json();
    assert_eq!(about.kind, ShareKind::Conversation);

    // The root's transcript, without the hidden inputs and the requests'
    // usage the conversation holds, and without its instructions.
    let root = page(&backend, &token, "").await;
    assert_eq!(root.start, 0);
    let held = kinds(&own.blocks);
    assert!(held.contains(&"context".to_owned()) && held.contains(&"response".to_owned()), "{held:?}");
    let expected: Vec<String> = held
        .into_iter()
        .filter(|kind| kind != "context" && kind != "response")
        .collect();
    assert_eq!(kinds(&root.blocks), expected);
    assert!(root.instructions.is_empty());
    let hidden = own.blocks.iter().find(|block| matches!(block, Block::Context(_))).unwrap();
    missed(
        &public(&backend, &token, &format!("/transcript/blocks/{}", hidden.id()), None).await,
        "a hidden block",
    );
    // A call reads whole with the output its view shows.
    let call = root
        .blocks
        .iter()
        .find(|block| matches!(block, Block::ToolCall(_)))
        .unwrap_or_else(|| panic!("the first call: {:?}", root.blocks));
    let whole = public(&backend, &token, &format!("/transcript/blocks/{}", call.id()), None).await;
    let WholeBlock { block: Block::ToolCall(whole), failures } = whole.json() else { panic!("a call") };
    assert!(failures.is_none());
    let Some(ToolView::Shell(view)) = &whole.view else { panic!("a shell call: {whole:?}") };
    assert!(view.chunks.iter().any(|chunk| chunk.text.contains("hello from the command")), "{view:?}");

    // The child's transcript, by the number its link names.
    let helper = page(&backend, &token, &format!("?node={child}")).await;
    assert!(texts(&helper.blocks).contains(&"hi from the child".to_owned()), "{:?}", helper.blocks);
    missed(&public(&backend, &token, "/transcript?node=99", None).await, "an agent it lacks");

    // The media its blocks show, and no other blob of the owner's.
    let image = public(&backend, &token, &format!("/blobs/{}?type=image/png", shot.sha256), None).await;
    assert_eq!(image.status, StatusCode::OK);
    assert_eq!(image.body, *PNG);
    missed(
        &public(&backend, &token, &format!("/blobs/{}", unsent.sha256), None).await,
        "a blob it does not show",
    );

    // A later message is not shared until the owner takes the snapshot again.
    scripts.root(FIRST, vec![say("later answer")]);
    socket.chat("m2", "One more").await;
    assert!(!texts(&page(&backend, &token, "").await.blocks).contains(&"later answer".to_owned()));
    let retaken = backend
        .put(&format!("/api/conversations/{FIRST}/shares/{token}"), &master, json!({}))
        .await;
    assert!(matches!(shared(&retaken), ShareAnswer::Conversation { token: same, .. } if same.as_str() == token));
    assert!(texts(&page(&backend, &token, "").await.blocks).contains(&"later answer".to_owned()));

    // Archiving keeps it; deleting the conversation removes it.
    let archived = backend
        .patch(&format!("/api/conversations/{FIRST}"), &master, json!({ "archived": true }))
        .await;
    assert_eq!(archived.status, StatusCode::OK, "{}", String::from_utf8_lossy(&archived.body));
    assert_eq!(public(&backend, &token, "", None).await.status, StatusCode::OK);
    let deleted = backend.delete(&format!("/api/conversations/{FIRST}"), &master).await;
    assert_eq!(deleted.status, StatusCode::NO_CONTENT);
    for path in ["", "/transcript", &format!("/blobs/{}", shot.sha256)] {
        missed(&public(&backend, &token, path, None).await, path);
    }
    backend.close().await;
}
