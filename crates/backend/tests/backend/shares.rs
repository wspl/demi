//! Sharing (`sharing.md`, `web-api-conversations.md` § Sharing): an answer's share holds
//! the message and the texts of the reply to it; a conversation's holds the
//! root's messages and texts, with the media they show, as they were when it
//! was taken until the owner takes it again, and in place of the agent's
//! work between them the number of its steps, never a step itself;
//! the public routes answer the token alone, ignore a session and answer
//! every miss alike; and a share goes with its conversation or its Stop
//! Sharing, at once.

use std::sync::Arc;

use demi_provider_common::testing::MockVendor;
use demi_conversation_socket_protocol::{ServerFrame, TranscriptPatch};
use demi_conversation_socket_protocol::{ClientContent, ClientFrame};
use demi_shared_types::{Block, MessageComment, SessionPhase, TurnId};
use demi_provider_common::testing::event;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::shares::{ShareAnswer, ShareKind, SharedConversation, SharedEntry, SharedTranscriptPage};
use demi_web_api_protocol::state::SyncEvent;
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{
    FIRST, Socket, anthropic, answer, choose, create, kinds, message, send, summary, text_block, tool_use_block,
    transcript,
};
use crate::subagents::{Scripts, say, shell, tree_on};
use crate::support::{Answer, Harness, Session, TestBackend};
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

async fn page(backend: &TestBackend, token: &str, query: &str) -> SharedTranscriptPage {
    let read = public(backend, token, &format!("/transcript{query}"), None).await;
    assert_eq!(read.status, StatusCode::OK, "{}", String::from_utf8_lossy(&read.body));
    read.json()
}

/// The blocks of `entries`.
fn blocks(entries: &[SharedEntry]) -> Vec<Block> {
    entries
        .iter()
        .filter_map(|entry| match entry {
            SharedEntry::Block(block) => Some(block.clone()),
            SharedEntry::StepsNotShared(_) => None,
        })
        .collect()
}

/// Each of `entries` as its block's type, or `steps N` for a run of steps
/// not shared.
fn entries(entries: &[SharedEntry]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| match entry {
            SharedEntry::Block(block) => kinds(std::slice::from_ref(block)).remove(0),
            SharedEntry::StepsNotShared(count) => format!("steps {count}"),
        })
        .collect()
}

/// The texts of `entries`.
fn texts(entries: &[SharedEntry]) -> Vec<String> {
    blocks(entries)
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
async fn an_answers_share_holds_its_message_and_reply_alone_until_the_owner_stops_sharing() {
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
    let own = transcript(&backend, &master, FIRST).await.blocks;
    let text = |wanted: &str| {
        own
            .iter()
            .find_map(|block| match block {
                Block::Text(text) if text.text == wanted => Some(text.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("no text {wanted}: {own:?}"))
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

    // Anyone reads the message and the texts of its reply, the call between
    // them as one step not shared, and nothing else.
    let about: SharedConversation = public(&backend, &token, "", None).await.json();
    assert_eq!(about.kind, ShareKind::Answer);
    assert_eq!(about.taken_at, taken_at);
    let read = page(&backend, &token, "").await;
    assert_eq!(entries(&read.blocks), ["user", "text", "steps 1", "text"]);
    assert_eq!(texts(&read.blocks), ["Looking it up.", "Answer one."]);
    let SharedEntry::Block(Block::User(question)) = &read.blocks[0] else { unreachable!() };
    assert_eq!(serde_json::to_value(&question.content).unwrap()[0]["text"], "Question one");
    let call = own.iter().find(|block| matches!(block, Block::ToolCall(_))).unwrap();
    missed(&public(&backend, &token, &format!("/transcript/blocks/{}", call.id()), None).await, "a call");
    // Another user's session changes nothing.
    let as_stranger = public(&backend, &token, "/transcript", Some(&stranger)).await;
    assert_eq!(as_stranger.json::<SharedTranscriptPage>(), read);

    // Any complete text of the conversation's agent is an answer, with the
    // message that starts its request; a message is none. An answer is
    // never taken again.
    let ShareAnswer::Answer { token: earlier, .. } =
        shared(&share(&backend, &master, json!({ "kind": "answer", "blockId": looking.id })).await)
    else {
        panic!("an answer's share")
    };
    assert_ne!(earlier.as_str(), token);
    let earlier = page(&backend, earlier.as_str(), "").await;
    assert_eq!(entries(&earlier.blocks), ["user", "text"]);
    assert_eq!(texts(&earlier.blocks), ["Looking it up."]);
    assert_eq!(blocks(&earlier.blocks)[0].id(), &question.id);
    let refusal = share(&backend, &master, json!({ "kind": "answer", "blockId": question.id })).await;
    assert_eq!(refusal.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::ShareTargetInvalid));
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
    let left = summary(&backend, &master, FIRST).await.shares;
    assert!(left.iter().all(|share| share.token().as_str() != token), "{left:?}");
    assert_eq!(left.len(), 1, "the earlier answer's share stays");
    backend.close().await;
}

/// The root's answer once its child reports.
const SAID: &str =
    "the helper said hi: ![The shot](attachment:a1), [the notes](attachment:a3), [a preview](attachment:a4-preview)";

/// The root's answer once its later child reports.
const AFTER_LATER: &str = "after the later child";

// A few seconds: the root thinks and runs three shell jobs on a real device,
// one of them the spawn of a child, which runs one.
#[tokio::test]
async fn a_conversations_share_shows_what_was_said_and_its_media_as_they_were_until_taken_again() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _paired, root, _provider) = tree_on(&scripts, Harness::new()).await;
    std::fs::write(format!("{root}/shown.png"), &*PNG).unwrap();
    std::fs::write(format!("{root}/notes.md"), "# Notes\n").unwrap();
    std::fs::write(format!("{root}/preview.md"), "# Preview\n").unwrap();
    // An image only the child uploads.
    let viewed = demi_agent_store::testing::png(4, 4, 2);
    std::fs::write(format!("{root}/viewed.png"), viewed.as_bytes()).unwrap();
    let stranger = backend.make_user(&master, "stranger@example.test", "user").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let shot = upload(&backend, &master, "shot.png", "image/png", &PNG).await;
    let unsent = demi_agent_store::testing::png(5, 5, 1);
    let unsent = upload(&backend, &master, "other.png", "image/png", unsent.as_bytes()).await;
    // Each child's first answer is held until its parent has answered and is
    // idle. A child that ended sooner, while its spawn command still ran,
    // would reach the parent before its next request, which takes the report
    // in (`subagents.md` § Delivery and scheduling); no request would then
    // ask for the words scripted after the report.
    let helper =
        scripts.child_held(vec![shell("c1", &format!("demi attachment upload {root}/viewed.png")), say("hi from the child")]);
    let mut thought = shell("t1", "printf 'hello from the command\\n' && demi attachment upload shown.png");
    thought.insert(0, event::thinking("The file is in the root."));
    scripts.root(
        FIRST,
        vec![
            thought,
            shell("t2", "demi agent spawn --description helper <<< 'Say hi'"),
            say("asked the helper"),
            // The child's completion wakes the idle parent.
            // a2 is the child's; a3 and a4 do not exist yet; `a4-preview`
            // is no attachment's name.
            say(SAID),
        ],
    );
    socket.send(&with_upload("m1", "Look at this", &[(&shot, "shot.png")])).await;
    socket.until_idle().await;
    helper.notify_one();
    // The root's answer after its child's report, and then its rest.
    socket
        .until(|frame| {
            matches!(frame, ServerFrame::TranscriptPatch { patches, .. } if patches.iter().any(|patch| match patch {
                TranscriptPatch::Add { value: Block::Text(text), .. }
                | TranscriptPatch::ReplaceBlock { value: Block::Text(text), .. } => text.text == SAID,
                TranscriptPatch::AppendText { delta, .. } => delta.contains("the helper said hi"),
                _ => false,
            }))
        })
        .await;
    socket
        .until(|frame| matches!(frame, ServerFrame::Phase { subagent_id: None, phase: SessionPhase::Idle }))
        .await;
    let own = transcript(&backend, &master, FIRST).await;
    let child = own.subagents[0].child.number;
    // The child's attachment, which no text of the root names.
    let viewed = backend.get(&format!("/api/conversations/{FIRST}/attachments/a2"), Some(&master)).await;
    assert_eq!(viewed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&viewed.body));
    let viewed = viewed.json::<demi_web_api_protocol::attachments::ConversationAttachment>().blob;

    let ShareAnswer::Conversation { token, .. } =
        shared(&share(&backend, &master, json!({ "kind": "conversation" })).await)
    else {
        panic!("a conversation's share")
    };
    let token = token.as_str().to_owned();
    let about: SharedConversation = public(&backend, &token, "", Some(&stranger)).await.json();
    assert_eq!(about.kind, ShareKind::Conversation);

    // The root's message and texts; in place of its thinking and calls,
    // their number, and nothing for the notice of the child's end between
    // its texts or the hidden inputs and usage the conversation holds.
    let root = page(&backend, &token, "").await;
    assert_eq!(root.start, 0);
    let held = kinds(&own.blocks);
    for kind in ["context", "response", "thinking", "tool_call"] {
        assert!(held.contains(&kind.to_owned()), "{kind} in {held:?}");
    }
    assert_eq!(entries(&root.blocks), ["user", "steps 3", "text", "text"], "{held:?}");
    assert_eq!(texts(&root.blocks), ["asked the helper", SAID]);
    assert!(root.instructions.is_empty());
    // No step, hidden block or subagent is reached, by id, by number or by
    // a blob only a step shows.
    for block in own.blocks.iter().filter(|block| !matches!(block, Block::User(_) | Block::Text(_))) {
        missed(
            &public(&backend, &token, &format!("/transcript/blocks/{}", block.id()), None).await,
            &format!("a {} block", kinds(std::slice::from_ref(block))[0]),
        );
    }
    for query in [format!("?node={child}"), "?node=99".to_owned()] {
        missed(&public(&backend, &token, &format!("/transcript{query}"), None).await, &format!("agent {query}"));
    }
    let said = own.blocks.iter().find(|block| matches!(block, Block::Text(_))).unwrap();
    missed(
        &public(&backend, &token, &format!("/transcript/blocks/{}?node={child}", said.id()), None).await,
        "a block asked of a subagent",
    );
    missed(&public(&backend, &token, "/attachments/a2", None).await, "a subagent's attachment");
    missed(&public(&backend, &token, &format!("/blobs/{viewed}"), None).await, "a subagent's blob");

    // The media its blocks show, and no other blob of the owner's.
    let image = public(&backend, &token, &format!("/blobs/{}?type=image/png", shot.sha256), None).await;
    assert_eq!(image.status, StatusCode::OK);
    assert_eq!(image.body, *PNG);
    missed(
        &public(&backend, &token, &format!("/blobs/{}", unsent.sha256), None).await,
        "a blob it does not show",
    );

    // An answer's share reads the attachment its text names, and its
    // bytes, and no other of the conversation's.
    let said = own
        .blocks
        .iter()
        .find(|block| matches!(block, Block::Text(text) if text.text == SAID))
        .unwrap();
    let ShareAnswer::Answer { token: answer, .. } =
        shared(&share(&backend, &master, json!({ "kind": "answer", "blockId": said.id() })).await)
    else {
        panic!("an answer's share")
    };
    let answer = answer.as_str().to_owned();
    let named = public(&backend, &answer, "/attachments/a1", None).await;
    assert_eq!(named.status, StatusCode::OK, "{}", String::from_utf8_lossy(&named.body));
    let named: demi_web_api_protocol::attachments::ConversationAttachment = named.json();
    assert_eq!(named.name, "shown.png");
    let bytes = public(&backend, &answer, &format!("/blobs/{}", named.blob), None).await;
    assert_eq!(bytes.body, *PNG);
    missed(&public(&backend, &answer, "/attachments/a3", None).await, "an attachment that did not exist");
    // A number past the column's range is no attachment, here and in the
    // conversation's own route.
    let past = "a9223372036854775808";
    missed(&public(&backend, &answer, &format!("/attachments/{past}"), None).await, "a number out of range");
    let own_past = backend
        .get(&format!("/api/conversations/{FIRST}/attachments/{past}"), Some(&master))
        .await;
    assert_eq!(own_past.refusal(), (StatusCode::NOT_FOUND, ErrorCode::NotFound));

    // A later message, the attachments it uploads and the subagent it
    // starts are not shared until the owner makes the list again: each, and
    // an index past the list, answers as a token that names no share.
    let length = page(&backend, &token, "").await.length;
    let later_helper = scripts.child_held(vec![say("hi from the later child")]);
    scripts.root(
        FIRST,
        vec![
            shell("t3", "demi attachment upload notes.md preview.md"),
            shell("t4", "demi agent spawn --description later <<< 'Say hi'"),
            say("later answer"),
            // The later child's completion wakes the idle parent.
            say(AFTER_LATER),
        ],
    );
    socket.send(&send("m2", "One more")).await;
    socket.until_idle().await;
    later_helper.notify_one();
    socket
        .until(|frame| {
            matches!(frame, ServerFrame::TranscriptPatch { patches, .. } if patches.iter().any(|patch| match patch {
                TranscriptPatch::Add { value: Block::Text(text), .. }
                | TranscriptPatch::ReplaceBlock { value: Block::Text(text), .. } => text.text == AFTER_LATER,
                TranscriptPatch::AppendText { delta, .. } => delta.contains("after the later child"),
                _ => false,
            }))
        })
        .await;
    socket
        .until(|frame| matches!(frame, ServerFrame::Phase { subagent_id: None, phase: SessionPhase::Idle }))
        .await;
    let later = transcript(&backend, &master, FIRST).await;
    let later_child = later.subagents[1].child.number;
    let later_answer = later
        .blocks
        .iter()
        .find(|block| matches!(block, Block::Text(text) if text.text == "later answer"))
        .unwrap();
    let edge = blocks(&root.blocks)[0].id().clone();
    assert!(!texts(&page(&backend, &token, "").await.blocks).contains(&"later answer".to_owned()));
    for (path, what) in [
        ("/attachments/a3".to_owned(), "an attachment uploaded since"),
        (format!("/transcript/blocks/{}", later_answer.id()), "a block written since"),
        (format!("/transcript?around={}", later_answer.id()), "a page around a block written since"),
        (format!("/transcript?before={length}&edge={edge}"), "an index past the list"),
        (format!("/transcript?before=1&edge={}", later_answer.id()), "a `before` anchor not on the list"),
        (format!("/transcript?after=0&edge={}", later_answer.id()), "an `after` anchor not on the list"),
        (format!("/transcript?node={later_child}"), "a subagent started since"),
    ] {
        missed(&public(&backend, &token, &path, None).await, what);
    }
    let retaken = backend
        .put(&format!("/api/conversations/{FIRST}/shares/{token}"), &master, json!({}))
        .await;
    assert!(matches!(shared(&retaken), ShareAnswer::Conversation { token: same, .. } if same.as_str() == token));
    assert!(texts(&page(&backend, &token, "").await.blocks).contains(&"later answer".to_owned()));
    missed(&public(&backend, &token, &format!("/transcript?node={later_child}"), None).await, "a subagent");
    let notes = public(&backend, &token, "/attachments/a3", None).await;
    assert_eq!(notes.status, StatusCode::OK, "{}", String::from_utf8_lossy(&notes.body));
    assert_eq!(notes.json::<demi_web_api_protocol::attachments::ConversationAttachment>().name, "notes.md");
    // A target with more after the number names no attachment.
    missed(&public(&backend, &token, "/attachments/a4", None).await, "a4-preview");

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

// Two Share requests at once, as the page's retry of a slow first one: both
// answer the one share, and only its list stays.
#[tokio::test]
async fn shares_asked_at_once_answer_one_link() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(answer(&["Answer one."], 10, 2));
    socket.chat("m1", "Question one").await;

    let target = json!({ "kind": "conversation" });
    let (first, second) =
        tokio::join!(share(&backend, &master, target.clone()), share(&backend, &master, target));
    let token = |answer: &Answer| match shared(answer) {
        ShareAnswer::Conversation { token, .. } => token.as_str().to_owned(),
        ShareAnswer::Answer { .. } => panic!("a conversation's share"),
    };
    assert_eq!(token(&first), token(&second));
    let file = harness.data_dir().join("conversations").join(format!("{FIRST}.sqlite"));
    let connection = rusqlite::Connection::open(file).unwrap();
    let lists: Vec<String> = connection
        .prepare("SELECT DISTINCT token FROM share_entries")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(lists, [token(&first)]);
    backend.close().await;
}

// A message sent with comments on the first answer (`comments-and-ask.md`):
// the share of its answer holds the message with its comments, which the
// shared page shows as quotes. A second of a mock vendor.
#[tokio::test]
async fn a_shared_message_keeps_the_comments_it_was_sent_with() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let provider = anthropic(&backend, &master, &vendor).await;
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    vendor.respond(answer(&["Upload in chunks of 4 MB."], 10, 2));
    socket.chat("m1", "Why do uploads fail?").await;
    let reply = transcript(&backend, &master, FIRST).await.blocks[1].id().clone();
    let comment = MessageComment {
        id: "c1".into(),
        block_id: reply,
        start: 0,
        at: 0,
        end: 25,
        quote: "Upload in chunks of 4 MB.".into(),
        picture: None,
        text: "4 MB feels large on 2G.".into(),
    };
    vendor.respond(answer(&["Starting at 1 MB."], 10, 2));
    socket
        .send(&ClientFrame::Send {
            message_id: TurnId::try_from("m2").unwrap(),
            content: vec![ClientContent::Text { text: "Otherwise go ahead".into() }],
            comments: vec![comment.clone()],
        })
        .await;
    socket.until_idle().await;
    let own = transcript(&backend, &master, FIRST).await.blocks;
    let second = own
        .iter()
        .rev()
        .find(|block| matches!(block, Block::Text(_)))
        .unwrap()
        .id()
        .clone();

    let ShareAnswer::Answer { token, .. } =
        shared(&share(&backend, &master, json!({ "kind": "answer", "blockId": second })).await)
    else {
        panic!("an answer's share")
    };
    let read = page(&backend, token.as_str(), "").await;
    let SharedEntry::Block(Block::User(message)) = &read.blocks[0] else {
        panic!("{:?}", read.blocks)
    };
    assert_eq!(message.comments, [comment]);
    backend.close().await;
}
