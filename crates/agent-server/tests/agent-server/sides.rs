//! Side questions at the tree's boundary: a side conversation from the root's
//! history through one of its replies, whose first request is the one the root
//! would send next with the side's message, and which the root never learns of.

use demi_agent_server::{side_instruction, testing::{ScriptedProviders, TestClient, agent_entries, entry_in}};
use demi_agent_store::{ClosePhase, SideOf};
use demi_conversation_socket_protocol::{AgentRole, ClientFrame, ServerFrame};
use demi_provider_common::{InferenceItem, InferenceRequest, Medium, UserPart, testing::{Turn, event}};
use demi_shared_types::{Block, BlockId, FileExtension, MediaSource, NodeId, Timestamp};
use demi_agent_store::testing::{MemoryTreeStore, model_reading};
use serde_json::json;

use std::{rc::Rc, time::Duration};

use crate::subagents::{closed_phase, fixture, held_said, refused, root, root_receipts, said};
use crate::support::{
        Fixture, Gate, Model, ScriptedShells, TestProduct, agent, conversation, scripted_host_server, held, request_text, send, streamed_then_held, texts, until,
    };

/// The brief the root's first message gives; the side's first request
/// carries it as the root's does, which routes the side to its script.
const FIRST: &str = "Explain the estimate card.";

fn side() -> NodeId {
    NodeId::try_from("side-1").unwrap()
}

/// The root's newest assistant text.
fn last_text(fixture: &Fixture) -> BlockId {
    fixture
        .server
        .tree(&root())
        .unwrap()
        .root()
        .session()
        .transcript()
        .blocks
        .iter()
        .rev()
        .find_map(|block| match block {
            Block::Text(text) => Some(text.id.clone()),
            _ => None,
        })
        .expect("the root replied")
}

fn asking(block: BlockId, question: &str) -> SideOf {
    SideOf {
        block,
        start: 10,
        end: 21,
        quote: "stale cache".into(),
        picture: None,
        question: question.into(),
        asked_at: Timestamp::UNIX_EPOCH,
    }
}

fn last_user_text(request: &InferenceRequest) -> String {
    match request.items.last() {
        Some(InferenceItem::UserMessage { content }) => texts(content),
        other => panic!("the request ends with {other:?}"),
    }
}

/// How many of `next`'s items come before the root's message `text`: what
/// a side question's first request shares with it, the told side questions
/// after the message being the root's alone.
fn before_message(next: &InferenceRequest, text: &str) -> usize {
    next.items
        .iter()
        .position(|item| matches!(item, InferenceItem::UserMessage { content } if texts(content) == text))
        .expect("the request carries the message")
}

/// The root replied to its first message once, and its tree is open.
async fn replied(model: &Model) -> (Fixture, TestClient<TestProduct>) {
    replied_reading(model, &[]).await
}

/// [`replied`] with a model that reads `reads`, as one reading pictures.
async fn replied_reading(model: &Model, reads: &[FileExtension]) -> (Fixture, TestClient<TestProduct>) {
    let fixture = fixture(model, TestProduct::default());
    if !reads.is_empty() {
        fixture.resolver.select(model_reading("stub", "test-model", reads));
    }
    let client = fixture.opened().await;
    client.send(send("m1", FIRST)).await;
    until(|| model.root_requests().len() == 1).await;
    let tree = fixture.server.tree(&root()).unwrap();
    until(|| tree.is_quiescent()).await;
    (fixture, client)
}

// The root answers, a side question asks about its words, then the root takes
// its next message.
//
// The side's request is the root's next request up to the root's own new
// message, byte for byte: the same system prompt, tools and items, so it reads
// the root's prompt-cache entry. It then ends with one user message of the
// quote, the instruction and the question.
//
// The root hears of it only with its next message, as what the user asked and
// was answered. No agent learns of it otherwise: no agent number, no lifecycle
// frame, no message.
//
// Two short trees: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_side_question_starts_from_the_roots_next_request_and_the_root_hears_only_what_was_asked() {
    let model = Model::default();
    model.root([said("The card reads a stale cache."), said("Noted.")]);
    model.child(FIRST, [said("Its TTL is an hour.")]);
    let (fixture, mut client) = replied(&model).await;
    let block = last_text(&fixture);
    // The page shows its card, which subscribes to its node.
    client.subscribe(&side()).await;

    fixture
        .server
        .ask(&conversation(), side(), asking(block, "Why is it stale?"))
        .await
        .expect("the side question starts");
    until(|| closed_phase(&fixture, &side()).is_some()).await;
    assert_eq!(
        closed_phase(&fixture, &side()),
        Some(ClosePhase::Completed)
    );
    client.send(send("m2", "Fix it.")).await;
    until(|| model.root_requests().len() == 2).await;

    let asked = model.requests_of(FIRST);
    assert_eq!(asked.len(), 1);
    let side_request = &asked[0];
    let next = &model.root_requests()[1];
    assert_eq!(side_request.system_prompt, next.system_prompt);
    assert_eq!(side_request.tools, next.tools);
    assert_eq!(side_request.model_id, next.model_id);
    // It names the root's session for the provider's cache and routing,
    // while its own session stays its own, and it extends what the root's
    // answered request carried, the reply's response included.
    assert_eq!(side_request.cache_session(), next.cache_session());
    assert_ne!(side_request.session_id, next.session_id);
    assert_eq!(side_request.prompt_cache, next.prompt_cache);
    let shared = before_message(next, "Fix it.");
    assert_eq!(side_request.items.len(), shared + 1);
    assert_eq!(side_request.items[..shared], next.items[..shared]);
    assert_eq!(
        last_user_text(side_request),
        format!("> stale cache\n\n{}\n\nWhy is it stale?", side_instruction(false))
    );
    // Its usage is the conversation's, under no agent.
    assert_eq!(side_request.agent, None);
    assert_eq!(next.agent, Some(0));

    // The root hears of it only as what the user asked, with its next
    // message, never its instruction or its steps.
    let root_text = request_text(next);
    assert!(root_text.contains("Q: Why is it stale?\nA: Its TTL is an hour."), "{root_text}");
    assert!(!root_text.contains(&side_instruction(false)), "{root_text}");
    assert!(root_receipts(&fixture).is_empty());
    let record = fixture.store.record(&side()).unwrap();
    assert_eq!((record.parent, record.side.is_some()), (Some(root()), true));
    let listed = agent(&fixture.server, &root(), "list", json!({})).await;
    assert!(!listed.stdout.contains("side-1"), "{listed:?}");
    assert!(!listed.stdout.contains("TTL"), "{listed:?}");
    let frames = client.received();
    // It is listed as a side question, never as a subagent.
    assert!(
        frames.iter().flat_map(agent_entries).all(|entry| !matches!(entry.role, AgentRole::Subagent(_))),
        "{frames:?}"
    );
    assert!(
        frames.iter().filter_map(|frame| entry_in(frame, &side())).all(|entry| matches!(entry.role, AgentRole::Side(_))),
        "{frames:?}"
    );
    // Its transcript went to the page that subscribes to its node.
    assert!(frames.iter().any(|frame| matches!(
        frame,
        ServerFrame::TranscriptReset { node, .. } if *node == side()
    )));
}

// A side question about a picture of the reply: its first message carries the
// picture as its quote, then the picture's instruction and the question, after
// the root's history as a question about words has it.
//
// One short tree: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_side_question_about_a_picture_carries_the_picture_as_its_quote() {
    let model = Model::default();
    model.root([said("Here is the chart.")]);
    model.child(FIRST, [said("The 3G one.")]);
    let (fixture, _client) = replied_reading(&model, &[FileExtension::Png]).await;
    let block = last_text(&fixture);
    let url = "https://example.com/chart.png";
    let of = SideOf {
        start: 0,
        end: 1,
        quote: "The chart".into(),
        picture: Some(MediaSource::Url { url: url.into() }),
        ..asking(block, "Which connection is this?")
    };

    fixture.server.ask(&conversation(), side(), of).await.expect("the side question starts");
    until(|| closed_phase(&fixture, &side()).is_some()).await;

    let asked = model.requests_of(FIRST);
    let Some(InferenceItem::UserMessage { content }) = asked[0].items.last() else {
        panic!("{:#?}", asked[0].items)
    };
    assert_eq!(
        content[..],
        [
            UserPart::Image(Medium::Url(url.into())),
            UserPart::Text(format!("{}\n\nWhich connection is this?", side_instruction(true))),
        ]
    );
}

// A side question runs while the root works, and the user's Stop of the
// root's turn leaves it running; its follow-up is its own next turn, after
// its own history. Runs a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_side_question_outlives_the_roots_stop_and_its_follow_up_continues_its_history() {
    let model = Model::default();
    let gate = Gate::new();
    model.root([said("The card reads a stale cache."), Turn::pending()]);
    model.child(FIRST, [held_said(&gate, "Its TTL is an hour."), said("Shorten it to a minute.")]);
    let (fixture, mut client) = replied(&model).await;
    let block = last_text(&fixture);
    client.send(send("m2", "Keep going.")).await;
    until(|| model.root_requests().len() == 2).await;

    fixture
        .server
        .ask(&conversation(), side(), asking(block, "Why is it stale?"))
        .await
        .unwrap();
    until(|| model.requests_of(FIRST).len() == 1).await;
    client.send(ClientFrame::Abort {}).await;
    client
        .next_until(|frame| matches!(frame, ServerFrame::AbortResult { .. }))
        .await;
    assert!(fixture.store.record(&side()).unwrap().closed.is_none());
    gate.open();
    until(|| closed_phase(&fixture, &side()).is_some()).await;

    fixture
        .server
        .ask_more(&conversation(), side(), "What should it be?".into())
        .await
        .expect("the follow-up is taken");
    until(|| model.requests_of(FIRST).len() == 2).await;
    let follow_up = &model.requests_of(FIRST)[1];
    assert_eq!(last_user_text(follow_up), "What should it be?");
    let text = request_text(follow_up);
    let first = text.find("Why is it stale?").unwrap();
    let answer = text.find("Its TTL is an hour.").unwrap();
    assert!(first < answer, "{text}");
    until(|| fixture.store.record(&side()).unwrap().closed.is_some()).await;
    assert_eq!(fixture.store.record(&side()).unwrap().round, 2);
    assert!(model.is_done());
}

// Stop ends the side question's turn and records the stop in its transcript,
// the root's turn is not touched, and a follow-up asks again from its history.
// Runs a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_stopped_side_question_records_its_stop_and_takes_a_follow_up() {
    let model = Model::default();
    model.root([said("The card reads a stale cache.")]);
    model.child(FIRST, [Turn::pending(), said("Shorten it to a minute.")]);
    let (fixture, _client) = replied(&model).await;
    let block = last_text(&fixture);
    fixture
        .server
        .ask(&conversation(), side(), asking(block, "Why is it stale?"))
        .await
        .unwrap();
    until(|| model.requests_of(FIRST).len() == 1).await;

    fixture.server.stop_side(&conversation(), side()).await.expect("the stop is taken");
    assert_eq!(closed_phase(&fixture, &side()), Some(ClosePhase::Aborted));
    let ended = fixture.store.checkpoint(&side()).unwrap().transcript;
    assert!(matches!(ended.last(), Some(Block::Abort(_))), "{ended:?}");
    // Nothing runs: another Stop is taken and records nothing more.
    fixture.server.stop_side(&conversation(), side()).await.expect("a second stop is taken");
    assert_eq!(fixture.store.checkpoint(&side()).unwrap().transcript.len(), ended.len());
    assert!(fixture.server.stop_side(&conversation(), NodeId::try_from("nobody").unwrap()).await.is_err());

    fixture
        .server
        .ask_more(&conversation(), side(), "What should it be?".into())
        .await
        .expect("the follow-up is taken");
    until(|| model.requests_of(FIRST).len() == 2).await;
    until(|| fixture.store.record(&side()).unwrap().closed.is_some()).await;
    assert!(model.is_done());
}

// A side question deleted while its first answer is on its way stops, and its
// record and its card go, and its entry is one change that the pages holding it
// take: one that held an earlier revision is told it since, a page that opens
// is not sent it. The root is never told of it, and a follow-up finds no such
// side question. Runs a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_deleted_side_question_is_gone_from_the_store_the_pages_and_the_roots_next_message() {
    let model = Model::default();
    model.root([said("The card reads a stale cache."), said("Noted.")]);
    model.child(FIRST, [Turn::pending()]);
    let (fixture, mut client) = replied(&model).await;
    let block = last_text(&fixture);
    client.subscribe(&side()).await;
    fixture
        .server
        .ask(&conversation(), side(), asking(block, "Why is it stale?"))
        .await
        .unwrap();
    until(|| model.requests_of(FIRST).len() == 1).await;
    let before = client.received();
    assert!(before.iter().filter_map(|frame| entry_in(frame, &side())).next().is_some(), "{before:?}");
    let held = fixture.server.tree(&root()).unwrap().agents_version();

    fixture.server.delete_side(&conversation(), side()).await.expect("the deletion is taken");
    assert!(fixture.store.record(&side()).is_none());
    // The deletion is one change, at the next revision, as any entry's.
    let after = client.received();
    let deleted: Vec<_> = after
        .iter()
        .filter(|frame| matches!(frame, ServerFrame::Agent { agent, .. } if agent.id == side() && agent.deleted))
        .collect();
    assert_eq!(deleted.len(), 1, "{after:?}");
    assert!(!after.iter().any(|frame| matches!(frame, ServerFrame::Agents { .. })), "{after:?}");
    // A page that held the revision before it is told the change since.
    client.send(ClientFrame::AgentsSince { held }).await;
    let answer = client.received();
    let Some(ServerFrame::Agents { since, agents, .. }) = answer.iter().find(|frame| matches!(frame, ServerFrame::Agents { .. })) else {
        panic!("{answer:?}")
    };
    assert!(since.is_some());
    assert!(agents.iter().any(|entry| entry.id == side() && entry.deleted), "{agents:?}");
    // A page that holds none is sent the agents, which list no deleted one.
    let mut fresh = TestClient::connect(&fixture.server, &conversation());
    fresh.open().await;
    let opened = fresh.received();
    assert!(opened.iter().flat_map(agent_entries).all(|entry| entry.id != side()), "{opened:?}");
    assert!(fixture.server.ask_more(&conversation(), side(), "Again?".into()).await.is_err());
    assert!(fixture.server.delete_side(&conversation(), side()).await.is_err());

    client.send(send("m2", "Fix it.")).await;
    until(|| model.root_requests().len() == 2).await;
    assert!(told_blocks(&fixture).await.is_empty());
}

// A side question leaves a command running and idles. Stop returns although the
// command's runner never answers, closes it, and the command's end, which the
// runner reports later, opens no turn: the model gets no further request.
// Under a second on the paused clock.
#[tokio::test(flavor = "local", start_paused = true)]
async fn the_end_of_a_command_a_stop_ended_wakes_no_turn_of_the_side_question() {
    let model = Model::default();
    model.root([said("The card reads a stale cache.")]);
    model.child(
        FIRST,
        [
            Turn::Events(vec![event::tool_call(
                "call-1",
                "shell",
                json!({"script": "serve", "description": "Start the server", "background": true}),
            )]),
            said("serving"),
            said("woken by the command's end"),
        ],
    );
    let providers = Rc::new(ScriptedProviders::default());
    providers.provide_runtime("stub", model.clone());
    let shells = Rc::new(ScriptedShells::default());
    let store = MemoryTreeStore::new();
    let server = scripted_host_server(providers, shells.clone(), store.clone());
    let mut page = TestClient::connect(&server, &conversation());
    page.open().await;
    page.send(send("m1", FIRST)).await;
    until(|| model.root_requests().len() == 1).await;
    until(|| server.tree(&root()).unwrap().is_quiescent()).await;
    let block = server
        .tree(&root())
        .unwrap()
        .root()
        .session()
        .transcript()
        .blocks
        .iter()
        .rev()
        .find_map(|block| match block {
            Block::Text(text) => Some(text.id.clone()),
            _ => None,
        })
        .expect("the root replied");
    server.ask(&conversation(), side(), asking(block, "Why is it stale?")).await.unwrap();
    // The command's call window passes on the paused clock.
    for _ in 0..30 {
        if model.requests_of(FIRST).len() == 2 {
            break;
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert_eq!(model.requests_of(FIRST).len(), 2);

    tokio::time::timeout(Duration::from_secs(60), server.stop_side(&conversation(), side()))
        .await
        .expect("a stop waits for no command's end")
        .expect("the stop is taken");
    assert_eq!(store.record(&side()).and_then(|record| record.closed).map(|close| close.phase), Some(ClosePhase::Aborted));

    // The runner reports the end, long after.
    shells.last().end();
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert_eq!(model.requests_of(FIRST).len(), 2);
    assert_eq!(store.record(&side()).and_then(|record| record.closed).map(|close| close.phase), Some(ClosePhase::Aborted));
}

// A side question asks `demi agent` to start a subagent, send a
// message and change its directory: each is refused in words that say a
// side question cannot, and nothing starts. Runs a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_side_question_starts_no_subagent_and_sends_no_message() {
    let model = Model::default();
    let gate = Gate::new();
    model.root([said("The card reads a stale cache.")]);
    model.child(FIRST, [held_said(&gate, "Its TTL is an hour.")]);
    let (fixture, _client) = replied(&model).await;
    let block = last_text(&fixture);
    fixture
        .server
        .ask(&conversation(), side(), asking(block, "Why is it stale?"))
        .await
        .unwrap();
    until(|| model.requests_of(FIRST).len() == 1).await;

    let spawned = agent(&fixture.server, &side(), "spawn", json!({ "prompt": "task alpha" })).await;
    assert!(refused(&spawned).contains("a side question cannot start subagents"), "{spawned:?}");
    let sent = agent(&fixture.server, &side(), "send", json!({ "id": "0", "message": "hi" })).await;
    assert!(refused(&sent).contains("a side question cannot send messages"), "{sent:?}");
    let moved = agent(&fixture.server, &side(), "cwd", json!({ "dir": "/tmp" })).await;
    assert!(refused(&moved).contains("a side question"), "{moved:?}");
    assert!(root_receipts(&fixture).is_empty());
    gate.open();
    until(|| closed_phase(&fixture, &side()).is_some()).await;
    assert_eq!(model.requests().len(), 2);
}

// The root answers, a compaction summarizes the answer, and then the user asks
// about the answer's words. The reply lives only in the summary, so the side
// question starts from the conversation's context as it stands, the summary,
// and its request is the root's next one up to the root's own message.
//
// One tree with a compaction: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_side_question_about_a_compacted_reply_starts_from_the_summary() {
    const SUMMARY: &str = "Summary: the card reads a stale cache.";
    let model = Model::default();
    model.root([
        Turn::Events(vec![event::text("The card reads a stale cache."), event::response(60_000, 0)]),
        Turn::Events(vec![event::text(SUMMARY), event::response(10, 0)]),
        said("Noted."),
    ]);
    // After the compaction, every request's first message is the summary's.
    const COMPACTED: &str = "The earlier part of this conversation was compacted.";
    model.child(COMPACTED, [said("Its TTL is an hour.")]);
    let (fixture, client) = replied(&model).await;
    let block = last_text(&fixture);
    client.send(ClientFrame::Compact { node: None }).await;
    let tree = fixture.server.tree(&root()).unwrap();
    until(|| model.root_requests().len() == 2).await;
    until(|| tree.is_quiescent()).await;

    fixture
        .server
        .ask(&conversation(), side(), asking(block, "Why is it stale?"))
        .await
        .expect("the side question starts");
    until(|| closed_phase(&fixture, &side()).is_some()).await;
    client.send(send("m2", "Fix it.")).await;
    until(|| model.root_requests().len() == 3).await;

    let side_request = &model.requests_of(COMPACTED)[0];
    let next = &model.root_requests()[2];
    let shared = before_message(next, "Fix it.");
    assert_eq!(side_request.items.len(), shared + 1);
    assert_eq!(side_request.items[..shared], next.items[..shared]);
    assert_eq!(side_request.prompt_cache, next.prompt_cache);
    let text = request_text(side_request);
    assert!(text.contains(SUMMARY), "{text}");
    assert!(last_user_text(side_request).ends_with("Why is it stale?"));
}

// A page goes away while a side question answers, the answer ends, and the page
// comes back: its open lists the side question as it ended, idle, though it is
// no longer live; its answer is its node's transcript, which the history
// routes read.
//
// A few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_page_that_comes_back_receives_a_side_question_that_ended_while_it_was_away() {
    let model = Model::default();
    let gate = Gate::new();
    model.root([said("The card reads a stale cache.")]);
    model.child(FIRST, [held_said(&gate, "Its TTL is an hour.")]);
    let (fixture, client) = replied(&model).await;
    let block = last_text(&fixture);
    fixture
        .server
        .ask(&conversation(), side(), asking(block, "Why is it stale?"))
        .await
        .unwrap();
    until(|| model.requests_of(FIRST).len() == 1).await;
    drop(client);

    gate.open();
    until(|| closed_phase(&fixture, &side()).is_some()).await;
    let mut returned = fixture.client();
    returned.open().await;
    let frames = returned.received();
    let phase = frames.iter().rev().find_map(|frame| entry_in(frame, &side()).map(|entry| entry.phase));
    assert_eq!(phase, Some(demi_shared_types::SessionPhase::Idle), "{frames:?}");
    let stored = fixture.store.rows(&side());
    assert!(
        stored.iter().any(|block| matches!(block, Block::Text(text) if text.text == "Its TTL is an hour.")),
        "{stored:?}"
    );
}

/// The `side_questions` context blocks of the root's transcript.
async fn told_blocks(fixture: &Fixture) -> Vec<String> {
    let session = fixture.server.tree(&root()).unwrap().root().session().clone();
    session
        .history()
        .await
        .unwrap()
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Context(context) if context.source == demi_shared_types::SIDE_QUESTIONS_SOURCE => {
                Some(context.text.clone())
            }
            _ => None,
        })
        .collect()
}

// A turn the user stopped is never told to the root, the question before its
// answer included; a later follow-up that answered is, as before. Runs a few
// milliseconds.
#[tokio::test(flavor = "local")]
async fn the_root_is_never_told_a_turn_the_user_stopped() {
    let model = Model::default();
    let gate = Gate::new();
    model.root([said("The card reads a stale cache."), said("Noted.")]);
    // The first turn writes half a sentence and is stopped there.
    model.child(
        FIRST,
        [
            streamed_then_held(&gate, vec![event::text("Its TTL is")], vec![event::response(1, 1)]),
            said("Yes, a minute."),
        ],
    );
    let (fixture, client) = replied(&model).await;
    let block = last_text(&fixture);
    fixture
        .server
        .ask(&conversation(), side(), asking(block, "Why is it stale?"))
        .await
        .unwrap();
    until(|| model.requests_of(FIRST).len() == 1).await;
    fixture.server.stop_side(&conversation(), side()).await.unwrap();
    fixture
        .server
        .ask_more(&conversation(), side(), "Can it be shorter?".into())
        .await
        .unwrap();
    until(|| model.requests_of(FIRST).len() == 2).await;
    until(|| closed_phase(&fixture, &side()).is_some() && fixture.store.record(&side()).unwrap().round == 2).await;

    client.send(send("m2", "Fix it.")).await;
    until(|| model.root_requests().len() == 2).await;
    let told = format!("{TOLD}\n\nOn \"stale cache\":\nQ: Can it be shorter?\nA: Yes, a minute.");
    assert_eq!(told_blocks(&fixture).await, [told]);
    assert!(!request_text(&model.root_requests()[1]).contains("Why is it stale?"));
}

const TOLD: &str = "The user asked these aside and was answered there; they show what the user was unsure of, for you to know, with no need to answer them again.";

// The user asks about the root's reply aside, then sends the root a message:
// its first request carries what the user asked and was answered, under the
// asked words.
//
// - A follow-up asked while that turn works reaches no request of the turn.
// - The next message carries the follow-up alone, and a message after it
//   nothing more.
// - After a compaction, the next message carries them all again.
//
// One tree, a few milliseconds.
#[tokio::test(flavor = "local")]
async fn the_root_hears_of_a_side_question_with_the_users_next_message_once_and_never_mid_turn() {
    let model = Model::default();
    let release = Gate::new();
    model.root([
        said("The card reads a stale cache."),
        held(&release, vec![event::tool_call("call-1", "shell", json!({})), event::response(1, 1)]),
        said("Done."),
        said("Noted."),
        Turn::Events(vec![event::text("Sure."), event::response(60_000, 0)]),
        said("Summary: the card reads a stale cache."),
        said("Right."),
    ]);
    model.child(FIRST, [said("Its TTL is an hour."), said("Yes, a minute.")]);
    let (fixture, client) = replied(&model).await;
    let block = last_text(&fixture);
    fixture
        .server
        .ask(&conversation(), side(), asking(block, "Why is it stale?"))
        .await
        .unwrap();
    until(|| closed_phase(&fixture, &side()).is_some()).await;

    client.send(send("m2", "Fix it.")).await;
    until(|| model.root_requests().len() == 2).await;
    let first = format!("{TOLD}\n\nOn \"stale cache\":\nQ: Why is it stale?\nA: Its TTL is an hour.");
    assert_eq!(told_blocks(&fixture).await, [first.clone()]);
    assert!(request_text(&model.root_requests()[1]).contains(&first));

    // A follow-up while the turn works.
    fixture
        .server
        .ask_more(&conversation(), side(), "Can it be shorter?".into())
        .await
        .unwrap();
    until(|| model.requests_of(FIRST).len() == 2).await;
    until(|| fixture.store.record(&side()).unwrap().round == 2 && closed_phase(&fixture, &side()).is_some()).await;
    release.open();
    until(|| model.root_requests().len() == 3).await;
    let tree = fixture.server.tree(&root()).unwrap();
    until(|| tree.is_quiescent()).await;
    assert!(!request_text(&model.root_requests()[2]).contains("Can it be shorter?"));
    assert_eq!(told_blocks(&fixture).await, [first.clone()]);

    client.send(send("m3", "And now?")).await;
    until(|| model.root_requests().len() == 4).await;
    let more = format!("{TOLD}\n\nOn \"stale cache\":\nQ: Can it be shorter?\nA: Yes, a minute.");
    assert_eq!(told_blocks(&fixture).await, [first.clone(), more.clone()]);
    until(|| tree.is_quiescent()).await;

    client.send(send("m4", "Thanks.")).await;
    until(|| model.root_requests().len() == 5).await;
    until(|| tree.is_quiescent()).await;
    assert_eq!(told_blocks(&fixture).await, [first.clone(), more.clone()]);

    // A compaction summarizes the blocks away: as every source, it tells
    // again, with the next message, what the summary may have left out.
    client.send(ClientFrame::Compact { node: None }).await;
    until(|| model.root_requests().len() == 6).await;
    until(|| tree.is_quiescent()).await;
    client.send(send("m5", "Go on.")).await;
    until(|| model.root_requests().len() == 7).await;
    let again = format!(
        "{TOLD}\n\nOn \"stale cache\":\nQ: Why is it stale?\nA: Its TTL is an hour.\nQ: Can it be shorter?\nA: Yes, a minute."
    );
    assert_eq!(told_blocks(&fixture).await, [first, more, again.clone()]);
    assert!(request_text(&model.root_requests()[6]).contains(&again));
}

// The user sends the root a message while a side question is still being
// answered, its answer half written: the root hears nothing of it then, and
// hears of it, answered, with the next message once it ended.
//
// One tree, a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_side_question_still_being_answered_is_told_only_once_answered() {
    let model = Model::default();
    let gate = Gate::new();
    model.root([said("The card reads a stale cache."), said("Noted."), said("Sure.")]);
    model.child(FIRST, [streamed_then_held(&gate, vec![event::text("Its TTL")], vec![event::text(" is an hour."), event::response(1, 1)])]);
    let (fixture, mut client) = replied(&model).await;
    let block = last_text(&fixture);
    client.subscribe(&side()).await;
    fixture
        .server
        .ask(&conversation(), side(), asking(block, "Why is it stale?"))
        .await
        .unwrap();
    until(|| model.requests_of(FIRST).len() == 1).await;
    let tree = fixture.server.tree(&root()).unwrap();
    // Its answer is half written: the page that shows its card has its
    // first words.
    client
        .next_until(|frame| matches!(frame, ServerFrame::TranscriptPatch { node, .. } if *node == side()) && format!("{frame:?}").contains("Its TTL"))
        .await;

    client.send(send("m2", "Fix it.")).await;
    until(|| model.root_requests().len() == 2).await;
    // The root's turn ends while the side question still works.
    until(|| {
        tree.root().session().transcript().blocks.last().is_some_and(|block| matches!(block, Block::Response(_)))
    })
    .await;
    assert!(told_blocks(&fixture).await.is_empty(), "{:?}", told_blocks(&fixture).await);
    assert!(!request_text(&model.root_requests()[1]).contains("Why is it stale?"));

    gate.open();
    until(|| closed_phase(&fixture, &side()).is_some()).await;
    client.send(send("m3", "And now?")).await;
    until(|| model.root_requests().len() == 3).await;
    assert_eq!(
        told_blocks(&fixture).await,
        [format!("{TOLD}\n\nOn \"stale cache\":\nQ: Why is it stale?\nA: Its TTL is an hour.")]
    );
}

// A side question's follow-up compacts its history, so its window starts after
// the question the user first asked: the root still hears every question and
// its answer, read from the side question's store.
//
// One tree, a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_side_question_whose_history_was_compacted_still_tells_what_it_was_asked_first() {
    let model = Model::default();
    model.root([said("The card reads a stale cache."), said("Noted.")]);
    model.child(
        FIRST,
        [
            Turn::Events(vec![event::text("Its TTL is an hour."), event::response(95_000, 0)]),
            said("Summary: its TTL is an hour."),
            said("Yes, a minute."),
        ],
    );
    let (fixture, client) = replied(&model).await;
    let block = last_text(&fixture);
    fixture
        .server
        .ask(&conversation(), side(), asking(block, "Why is it stale?"))
        .await
        .unwrap();
    until(|| closed_phase(&fixture, &side()).is_some()).await;
    fixture
        .server
        .ask_more(&conversation(), side(), "Can it be shorter?".into())
        .await
        .unwrap();
    until(|| fixture.store.record(&side()).unwrap().round == 2 && closed_phase(&fixture, &side()).is_some()).await;
    let loaded = fixture.store.checkpoint(&side()).unwrap();
    assert!(
        !loaded.transcript.iter().any(|block| matches!(block, Block::User(user) if user.turn_id.as_str() == side().as_str())),
        "the side question's window starts after its first question"
    );

    client.send(send("m2", "Fix it.")).await;
    until(|| model.root_requests().len() == 2).await;
    assert_eq!(
        told_blocks(&fixture).await,
        [format!(
            "{TOLD}\n\nOn \"stale cache\":\nQ: Why is it stale?\nA: Its TTL is an hour.\nQ: Can it be shorter?\nA: Yes, a minute."
        )]
    );
}
