//! Compaction (`compaction.md`): passes before a turn, inside a turn and
//! before a model switch, the session copy that writes the summary, and the
//! `compact` action.

use demi_core::{AgentMessage, AgentMessageEvent, BlockId, Sender};
use demi_provider::ProviderFailure;

use super::*;
use crate::{
    session::compaction::COMPACTION_SUMMARY_INSTRUCTION, transcript::estimate::block_tokens,
};

/// A model whose window makes 800 tokens the threshold.
fn small_model() -> ModelSelection {
    let mut model = model_of("stub", "small-model");
    model.model.context_window = 1_000;
    model
}

/// A session whose model has a 1,000-token window and which keeps the last
/// 100 estimated tokens.
async fn small_session(
    provider: &ScriptedRuntime,
    tools: Vec<(String, Invoke)>,
    store: &Rc<MemoryTreeStore>,
) -> AgentSession {
    let session = start(
        provider,
        tools,
        store,
        SessionConfig {
            compaction: CompactionConfig {
                keep_recent_tokens: 100,
                threshold_percent: Some(80),
            },
            ..SessionConfig::default()
        },
    )
    .await;
    session
        .update_model(ModelSwitch {
            model: small_model(),
            runtime: None,
            apply: ModelSwitchApply::NextTurn,
        })
        .unwrap();
    session
}

/// An answer whose usage anchors nothing, so the estimate is the blocks'.
fn answer(text: &str) -> Turn {
    Turn::Events(vec![event::text(text), event::response(0, 0)])
}

/// A 750-token message.
fn long_message() -> Vec<demi_core::UserContentBlock> {
    text(&"x".repeat(3_000))
}

fn is_copy(request: &demi_provider::InferenceRequest) -> bool {
    request.session_id != "root"
}

#[tokio::test(flavor = "local")]
async fn a_history_over_the_threshold_is_compacted_before_the_turn_by_a_copy_that_repeats_its_prefix()
 {
    let provider = ScriptedRuntime::new([
        answer("first answer"),
        answer("  summary of the first turn\n"),
        answer("second answer"),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session(&provider, Vec::new(), &store).await;
    let phases = Rc::new(RefCell::new(Vec::new()));
    let _listener = session.subscribe({
        let phases = phases.clone();
        move |event| {
            if let SessionEvent::PhaseChanged { phase } = event {
                phases.borrow_mut().push(*phase);
            }
        }
    });
    session
        .send(long_message(), turn("t1"))
        .unwrap()
        .await
        .unwrap();

    session
        .send(text(&"y".repeat(400)), turn("t2"))
        .unwrap()
        .await
        .unwrap();

    let requests = provider.requests();
    let [first, summary, second] = requests.as_slice() else {
        panic!("{requests:?}");
    };
    // The copy's request is the session's history up to the cut, as the
    // session replays it, then the instruction; same system prompt, tools
    // and model.
    assert!(is_copy(summary) && !is_copy(second));
    let mut expected = first.items.to_vec();
    expected.push(InferenceItem::AssistantText {
        model_id: "small-model".into(),
        text: "first answer".into(),
    });
    expected.push(InferenceItem::UserMessage {
        content: text(COMPACTION_SUMMARY_INSTRUCTION),
    });
    assert_eq!(summary.items.as_ref(), expected.as_slice());
    assert_eq!(summary.system_prompt, second.system_prompt);
    assert_eq!(summary.model_id, "small-model");
    // The kept history begins with the boundary, and the marker ends it.
    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        [
            "user",
            "text",
            "response",
            "compaction_boundary",
            "user",
            "compaction_marker",
            "text",
            "response"
        ]
    );
    let Block::CompactionBoundary(boundary) = &blocks[3] else {
        unreachable!();
    };
    assert_eq!(boundary.summary, "summary of the first turn");
    let Block::CompactionMarker(marker) = &blocks[5] else {
        unreachable!();
    };
    assert_eq!(marker.boundary_id, boundary.id);
    assert_eq!(
        marker.compacted_tokens,
        blocks[..3].iter().map(block_tokens).sum::<u64>()
    );
    assert_eq!(item_kinds(&second.items), ["user_message", "user_message"]);
    assert_eq!(
        second.items[0],
        InferenceItem::UserMessage {
            content: text("Previous conversation summary:\nsummary of the first turn"),
        }
    );
    assert!(phases.borrow().contains(&SessionPhase::Compacting));
    // The copy saved nothing and its runtime was closed.
    assert!(store.saves().iter().all(|(node, _)| node == &root()));
    assert_eq!(provider.closes(), 1);
}

#[tokio::test(flavor = "local")]
async fn a_summary_request_that_exceeds_the_context_is_retried_with_half_the_window() {
    let too_long = Turn::Events(vec![ProviderEvent::Error(ProviderFailure {
        message: "prompt is too long".into(),
        code: Some(ErrorCode::ContextLengthExceeded),
        diagnostics: None,
        retry_after: None,
    })]);
    let provider = ScriptedRuntime::new([
        answer("one"),
        answer("two"),
        too_long,
        answer("summary of the first half"),
        answer("done"),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session(&provider, Vec::new(), &store).await;
    session
        .send(text(&"a".repeat(1_200)), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    session
        .send(text(&"b".repeat(1_200)), turn("t2"))
        .unwrap()
        .await
        .unwrap();

    session
        .send(text(&"c".repeat(800)), turn("t3"))
        .unwrap()
        .await
        .unwrap();

    let copies: Vec<usize> = provider
        .requests()
        .iter()
        .filter(|request| is_copy(request))
        .map(|request| request.items.len())
        .collect();
    // The first summary request held six blocks' items and the instruction,
    // the second the first half.
    assert_eq!(copies, [5, 3]);
    let blocks = session.transcript().blocks;
    let boundaries = blocks
        .iter()
        .filter(|block| matches!(block, Block::CompactionBoundary(_)))
        .count();
    assert_eq!(boundaries, 1);
    assert!(
        matches!(&blocks[3], Block::CompactionBoundary(_)),
        "{:?}",
        kinds(&blocks)
    );
}

#[tokio::test(flavor = "local")]
async fn a_blank_summary_compacts_nothing_and_a_failed_one_fails_the_action_without_a_boundary() {
    let provider = ScriptedRuntime::new([
        answer("first"),
        answer("   "),
        answer("second"),
        Turn::Events(vec![ProviderEvent::Error(ProviderFailure {
            message: "the key expired".into(),
            code: Some(ErrorCode::AuthExpired),
            diagnostics: None,
            retry_after: None,
        })]),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session(&provider, Vec::new(), &store).await;
    session
        .send(long_message(), turn("t1"))
        .unwrap()
        .await
        .unwrap();

    session
        .send(text(&"y".repeat(400)), turn("t2"))
        .unwrap()
        .await
        .unwrap();
    let failed = session
        .send(text(&"z".repeat(400)), turn("t3"))
        .unwrap()
        .await;

    assert_eq!(failed.unwrap_err().code.as_deref(), Some("auth_expired"));
    let blocks = session.transcript().blocks;
    assert!(
        !blocks.iter().any(|block| matches!(
            block,
            Block::CompactionBoundary(_) | Block::CompactionMarker(_)
        )),
        "{:?}",
        kinds(&blocks)
    );
    assert_eq!(
        kinds(&blocks),
        [
            "user", "text", "response", "user", "text", "response", "user"
        ]
    );
}

#[tokio::test(flavor = "local")]
async fn stop_during_a_summary_closes_the_copy_and_ends_the_action_as_stopped() {
    let provider = ScriptedRuntime::new([answer("first"), Turn::pending()]);
    let store = MemoryTreeStore::new();
    let session = small_session(&provider, Vec::new(), &store).await;
    session
        .send(long_message(), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    let running = session.send(text(&"y".repeat(400)), turn("t2")).unwrap();
    until(|| provider.requests().len() == 2).await;
    assert_eq!(session.phase(), SessionPhase::Compacting);

    let stopped = session.abort().await;

    assert_eq!(stopped.target, Some(AbortTarget::ActiveCompaction));
    assert_eq!(running.await, Ok(ActionEnd::Aborted));
    assert_eq!(
        kinds(&session.transcript().blocks),
        ["user", "text", "response", "user", "abort"]
    );
    assert_eq!(provider.closes(), 1);
}

#[tokio::test(flavor = "local")]
async fn a_tool_the_copy_calls_runs_without_changing_the_session() {
    let provider = ScriptedRuntime::new([
        answer("first"),
        Turn::Events(vec![
            event::tool_call("copy-call", "note", json!({})),
            event::response(0, 0),
        ]),
        answer("summary after the tool"),
        answer("second"),
    ]);
    let store = MemoryTreeStore::new();
    let runs = Rc::new(RefCell::new(0));
    let note = tool("note", {
        let runs = runs.clone();
        move |_| {
            *runs.borrow_mut() += 1;
            Box::pin(async { Ok(output("noted")) })
        }
    });
    let session = small_session(&provider, vec![note], &store).await;
    session
        .send(long_message(), turn("t1"))
        .unwrap()
        .await
        .unwrap();

    session
        .send(text(&"y".repeat(400)), turn("t2"))
        .unwrap()
        .await
        .unwrap();

    assert_eq!(*runs.borrow(), 1);
    let blocks = session.transcript().blocks;
    assert!(
        !blocks
            .iter()
            .any(|block| matches!(block, Block::ToolCall(_)))
    );
    let Block::CompactionBoundary(boundary) = &blocks[3] else {
        panic!("{:?}", kinds(&blocks));
    };
    assert_eq!(boundary.summary, "summary after the tool");
}

#[tokio::test(flavor = "local")]
async fn a_turn_whose_usage_keeps_reaching_the_threshold_compacts_at_most_three_times() {
    // Every response reports 900 tokens, over the threshold of 800: each
    // round of tools is followed by a pass, until the third.
    let round = |call: &str| {
        Turn::Events(vec![
            event::tool_call(call, "work", json!({})),
            event::response(900, 0),
        ])
    };
    let provider = ScriptedRuntime::new([
        answer("first"),
        round("c1"),
        answer("summary one"),
        round("c2"),
        answer("summary two"),
        round("c3"),
        answer("summary three"),
        round("c4"),
        Turn::Events(vec![event::text("done"), event::response(900, 0)]),
    ]);
    let store = MemoryTreeStore::new();
    let work = tool("work", |_| Box::pin(async { Ok(output(&"w".repeat(800))) }));
    let session = small_session(&provider, vec![work], &store).await;
    session
        .send(long_message(), turn("t1"))
        .unwrap()
        .await
        .unwrap();

    session
        .send(text("go on"), turn("t2"))
        .unwrap()
        .await
        .unwrap();

    let blocks = session.transcript().blocks;
    let count = |kind: &str| kinds(&blocks).iter().filter(|each| *each == kind).count();
    assert_eq!(count("compaction_boundary"), 3);
    assert_eq!(count("resume"), 3);
    assert_eq!(provider.remaining(), 0);
}

#[tokio::test(flavor = "local")]
async fn a_switch_to_a_smaller_window_compacts_with_the_model_before_it() {
    let provider = ScriptedRuntime::new([
        answer("first"),
        answer("second"),
        answer("summary by the large model"),
        answer("third"),
        answer("fourth"),
    ]);
    let store = MemoryTreeStore::new();
    let config = SessionConfig {
        compaction: CompactionConfig {
            keep_recent_tokens: 100,
            threshold_percent: Some(80),
        },
        ..SessionConfig::default()
    };
    let session = start(&provider, Vec::new(), &store, config).await;
    session
        .send(text(&"a".repeat(1_600)), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    session
        .send(text(&"b".repeat(1_600)), turn("t2"))
        .unwrap()
        .await
        .unwrap();
    session
        .update_model(ModelSwitch {
            model: small_model(),
            runtime: None,
            apply: ModelSwitchApply::NextTurn,
        })
        .unwrap();

    session
        .send(text("short"), turn("t3"))
        .unwrap()
        .await
        .unwrap();
    // A switch back to the larger window compacts nothing.
    session
        .update_model(ModelSwitch {
            model: test_model(),
            runtime: None,
            apply: ModelSwitchApply::NextTurn,
        })
        .unwrap();
    session
        .send(text("more"), turn("t4"))
        .unwrap()
        .await
        .unwrap();

    let requests = provider.requests();
    let models: Vec<(&str, bool)> = requests
        .iter()
        .map(|request| (request.model_id.as_str(), is_copy(request)))
        .collect();
    assert_eq!(
        models,
        [
            ("test-model", false),
            ("test-model", false),
            ("test-model", true),
            ("small-model", false),
            ("test-model", false)
        ]
    );
}

#[tokio::test(flavor = "local")]
async fn the_compact_action_writes_a_steer_that_came_during_it_and_runs_a_turn_only_for_agent_messages()
 {
    let (summary, release) = gated_turn(vec![event::text("the summary"), event::response(0, 0)]);
    let (second_summary, second_release) =
        gated_turn(vec![event::text("another summary"), event::response(0, 0)]);
    let provider = ScriptedRuntime::new([
        answer("first"),
        answer("second"),
        summary,
        answer("third"),
        second_summary,
        answer("read the message"),
    ]);
    let store = MemoryTreeStore::new();
    let config = SessionConfig {
        compaction: CompactionConfig {
            keep_recent_tokens: 100,
            threshold_percent: Some(80),
        },
        ..SessionConfig::default()
    };
    let session = start(&provider, Vec::new(), &store, config).await;
    session
        .send(long_message(), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    session
        .send(long_message(), turn("t2"))
        .unwrap()
        .await
        .unwrap();

    let compacting = session.compact().unwrap();
    until(|| provider.requests().len() == 3).await;
    session
        .steer(text("mind the tests"), BlockId::try_from("s1").unwrap())
        .unwrap();
    let _ = release.send(());
    compacting.await.unwrap();

    // The steer waits in the transcript; no turn ran for it.
    let blocks = session.transcript().blocks;
    assert_eq!(kinds(&blocks).last().map(String::as_str), Some("steer"));
    assert!(
        blocks
            .iter()
            .any(|block| matches!(block, Block::CompactionBoundary(_)))
    );
    assert_eq!(provider.requests().len(), 3);

    session
        .send(long_message(), turn("t3"))
        .unwrap()
        .await
        .unwrap();
    let again = session.compact().unwrap();
    until(|| provider.requests().len() == 5).await;
    let message = AgentMessage {
        id: BlockId::try_from("m1").unwrap(),
        sender: Sender {
            id: NodeId::try_from("child").unwrap(),
            description: "worker".into(),
            round: 1,
        },
        recipient_id: root(),
        timestamp: Timestamp::UNIX_EPOCH,
        content: "found it".into(),
        event: AgentMessageEvent::Message {},
    };
    session.accept_agent_message(message).await.unwrap();
    let _ = second_release.send(());
    again.await.unwrap();

    // The waiting message reached the model after the pass.
    let requests = provider.requests();
    assert_eq!(requests.len(), 6);
    assert!(!is_copy(&requests[5]));
    assert_eq!(
        kinds(&session.transcript().blocks)
            .last()
            .map(String::as_str),
        Some("response")
    );
}
