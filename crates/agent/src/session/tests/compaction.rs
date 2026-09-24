//! Compaction (`compaction.md`): passes before a turn, inside a turn and
//! before a model switch, the session copy that writes the summary, and the
//! `compact` action.

use demi_core::{AgentMessage, AgentMessageEvent, BlockId, QueuedMessage, Sender, TokenUsage};
use demi_provider::ProviderFailure;
use futures_util::StreamExt;

use super::*;
use crate::{
    session::compaction::COMPACTION_SUMMARY_INSTRUCTION,
    transcript::{RESUME_TEXT, estimate::block_tokens},
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
    let compaction = CompactionConfig {
        keep_recent_tokens: 100,
        threshold_percent: Some(80),
    };
    small_session_with(provider, tools, store, compaction).await
}

/// A session whose model has a 1,000-token window and which compacts as
/// `compaction` says.
async fn small_session_with(
    provider: &ScriptedRuntime,
    tools: Vec<(String, Invoke)>,
    store: &Rc<MemoryTreeStore>,
    compaction: CompactionConfig,
) -> AgentSession {
    let config = SessionConfig {
        compaction,
        ..SessionConfig::default()
    };
    let session = start(provider, tools, store, config).await;
    session
        .update_model(ModelSwitch {
            model: small_model(),
            runtime: None,
            apply: ModelSwitchApply::NextTurn,
        })
        .unwrap();
    session
}

/// Compaction that keeps the last `keep_recent_tokens` and runs only when
/// asked, so the history written with it can grow over any threshold.
fn only_when_asked(keep_recent_tokens: u64) -> CompactionConfig {
    CompactionConfig {
        keep_recent_tokens,
        threshold_percent: None,
    }
}

/// The node `root` restored from a copy of `store`, compacting at 80% of its
/// model's window and keeping the last `keep_recent_tokens`: a history
/// written without compaction meets a threshold it is over.
fn restore_compacting(
    store: &MemoryTreeStore,
    provider: &ScriptedRuntime,
    keep_recent_tokens: u64,
) -> AgentSession {
    let copy = store.copy();
    let checkpoint = copy.checkpoint(&root()).unwrap();
    let config = SessionConfig {
        compaction: CompactionConfig {
            keep_recent_tokens,
            threshold_percent: Some(80),
        },
        ..SessionConfig::default()
    };
    let (session, _) = restore_configured(
        checkpoint,
        &copy,
        provider,
        test_runtime(Vec::new()),
        Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
        config,
    );
    session
}

/// An answer whose usage anchors nothing, so the estimate is the blocks'.
fn answer(text: &str) -> Turn {
    Turn::Events(vec![event::text(text), event::response(0, 0)])
}

/// A run that streams `partial` and then waits until it is stopped.
fn partial_then_hang(partial: &str) -> Turn {
    let delta = event::text(partial);
    Turn::Stream(Box::new(move |_| {
        futures_util::stream::iter([delta])
            .chain(futures_util::stream::pending())
            .boxed_local()
    }))
}

/// A summary request that exceeds the model's context.
fn too_long() -> Turn {
    Turn::Events(vec![ProviderEvent::Error(ProviderFailure {
        message: "prompt is too long".into(),
        code: Some(ErrorCode::ContextLengthExceeded),
        diagnostics: None,
        retry_after: None,
    })])
}

/// A 750-token message.
fn long_message() -> Vec<demi_core::UserContentBlock> {
    text(&"x".repeat(3_000))
}

fn is_copy(request: &demi_provider::InferenceRequest) -> bool {
    request.session_id != "root"
}

/// How many items each summary request held, in order.
fn summary_sizes(provider: &ScriptedRuntime) -> Vec<usize> {
    provider
        .requests()
        .iter()
        .filter(|request| is_copy(request))
        .map(|request| request.items.len())
        .collect()
}

fn user_item(message: &str) -> InferenceItem {
    InferenceItem::UserMessage {
        content: text(message),
    }
}

/// What the model receives of a `compaction_boundary` holding `summary`.
fn summary_item(summary: &str) -> InferenceItem {
    user_item(&format!("Previous conversation summary:\n{summary}"))
}

fn answer_item(model_id: &str, answer: &str) -> InferenceItem {
    InferenceItem::AssistantText {
        model_id: model_id.into(),
        text: answer.into(),
    }
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
    expected.push(answer_item("small-model", "first answer"));
    expected.push(user_item(COMPACTION_SUMMARY_INSTRUCTION));
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
    assert_eq!(second.items[0], summary_item("summary of the first turn"));
    assert!(phases.borrow().contains(&SessionPhase::Compacting));
    // The copy saved nothing and its runtime was closed.
    assert!(store.saves().iter().all(|(node, _)| node == &root()));
    assert_eq!(provider.closes(), 1);
}

#[tokio::test(flavor = "local")]
async fn a_summary_request_that_exceeds_the_context_is_retried_with_half_the_window() {
    let provider = ScriptedRuntime::new([
        answer("one"),
        answer("two"),
        too_long(),
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

    // The first summary request held six blocks' items and the instruction,
    // the second the first half.
    assert_eq!(summary_sizes(&provider), [5, 3]);
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
    // The turn's own request never went out.
    assert_eq!(provider.requests().len(), 2);
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

#[tokio::test(flavor = "local")]
async fn a_summary_that_exceeds_the_context_down_to_one_block_fails_the_action_and_changes_nothing()
{
    let provider = ScriptedRuntime::new([
        answer("old answer"),
        answer("recent answer"),
        too_long(),
        too_long(),
        answer("recovered"),
        answer("summary"),
        answer("fourth answer"),
        too_long(),
        too_long(),
        too_long(),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session_with(&provider, Vec::new(), &store, only_when_asked(100)).await;
    let errors = Rc::new(RefCell::new(Vec::new()));
    let _listener = session.subscribe({
        let errors = errors.clone();
        move |event| {
            if let SessionEvent::Error { report } = event {
                errors.borrow_mut().push(report.clone());
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
    let before = session.transcript();

    let failed = session.compact().unwrap().await;

    // The window's three blocks, then its first alone: the overflow fails
    // the action, reported once, and the history is as it was.
    let failure = failed.unwrap_err();
    assert_eq!(failure.code.as_deref(), Some("context_length_exceeded"));
    assert_eq!(*errors.borrow(), [*failure]);
    assert_eq!(summary_sizes(&provider), [3, 2]);
    assert_eq!(session.transcript(), before);
    // The session goes on.
    session
        .send(text("recover"), turn("t3"))
        .unwrap()
        .await
        .unwrap();

    // After a boundary, the window shrinks down to the boundary and the
    // block after it, and then fails the same way.
    session.compact().unwrap().await.unwrap();
    session
        .send(text(&"z".repeat(400)), turn("t4"))
        .unwrap()
        .await
        .unwrap();
    let before = session.transcript();

    let failed = session.compact().unwrap().await;

    assert_eq!(
        failed.unwrap_err().code.as_deref(),
        Some("context_length_exceeded")
    );
    assert_eq!(summary_sizes(&provider), [3, 2, 3, 6, 4, 3]);
    assert_eq!(session.transcript(), before);
}

#[tokio::test(flavor = "local")]
async fn a_retry_over_the_threshold_compacts_before_it_reruns_and_a_stop_during_that_pass_leaves_no_boundary()
 {
    let question = format!("old question {}", "x".repeat(3_200));
    let provider = ScriptedRuntime::new([
        answer("old answer"),
        answer("bad answer"),
        // The first restored session: the summary, then the rerun.
        answer("retry summary"),
        answer("retried answer"),
        // The second: a summary that ends only when it is stopped.
        Turn::pending(),
    ]);
    let store = MemoryTreeStore::new();
    let written = small_session_with(&provider, Vec::new(), &store, only_when_asked(1)).await;
    written
        .send(text(&question), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    written
        .send(text("retry this"), turn("t2"))
        .unwrap()
        .await
        .unwrap();
    let session = restore_compacting(&store, &provider, 1);

    session.retry().unwrap().await.unwrap();

    // Rewound to its message, the history is over the threshold: the pass
    // runs before the rerun, whose request starts with the summary.
    let requests = provider.requests();
    let [summary, rerun] = &requests[2..] else {
        panic!("{requests:?}");
    };
    assert_eq!(
        summary.items.as_ref(),
        [
            user_item(&question),
            answer_item("small-model", "old answer"),
            user_item(COMPACTION_SUMMARY_INSTRUCTION),
        ]
    );
    assert_eq!(
        rerun.items.as_ref(),
        [summary_item("retry summary"), user_item("retry this")]
    );
    assert_eq!(rerun.turn_id, "t2");
    assert_eq!(
        kinds(&session.transcript().blocks),
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

    let stopped_session = restore_compacting(&store, &provider, 1);
    let retrying = stopped_session.retry().unwrap();
    until(|| provider.requests().len() == 5).await;
    let stopped = stopped_session.abort().await;

    assert_eq!(stopped.target, Some(AbortTarget::ActiveCompaction));
    assert_eq!(retrying.await, Ok(ActionEnd::Aborted));
    // The rewind stays, with the stop and no boundary; the rerun's request
    // never went out.
    assert_eq!(
        kinds(&stopped_session.transcript().blocks),
        ["user", "text", "response", "user", "abort"]
    );
    assert_eq!(provider.requests().len(), 5);
}

#[tokio::test(flavor = "local")]
async fn a_resume_over_the_threshold_summarizes_the_stopped_progress_before_it_continues_and_a_stop_during_that_pass_leaves_no_boundary()
 {
    let question = format!("old question {}", "x".repeat(3_000));
    let partial = format!("partial answer {}", "y".repeat(300));
    let provider = ScriptedRuntime::new([
        partial_then_hang(&partial),
        // The first restored session: the summary, then the continuation.
        answer("resume summary"),
        answer("continued after the summary"),
        // The second: a summary that ends only when it is stopped.
        Turn::pending(),
    ]);
    let store = MemoryTreeStore::new();
    let written = small_session_with(&provider, Vec::new(), &store, only_when_asked(1)).await;
    let running = written.send(text(&question), turn("t1")).unwrap();
    until(|| written.transcript().blocks.len() == 2).await;
    written.abort().await;
    running.await.unwrap();
    let session = restore_compacting(&store, &provider, 1);

    session.resume().unwrap().await.unwrap();

    // The summary holds what the stopped turn wrote, and the model continues
    // after it.
    let requests = provider.requests();
    let [summary, continuation] = &requests[1..] else {
        panic!("{requests:?}");
    };
    assert_eq!(
        summary.items.as_ref(),
        [
            user_item(&question),
            answer_item("small-model", &partial),
            user_item(COMPACTION_SUMMARY_INSTRUCTION),
        ]
    );
    assert_eq!(
        continuation.items.as_ref(),
        [summary_item("resume summary"), user_item(RESUME_TEXT)]
    );
    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        [
            "user",
            "text",
            "abort",
            "compaction_boundary",
            "resume",
            "compaction_marker",
            "text",
            "response"
        ]
    );
    assert!(matches!(&blocks[2], Block::Abort(abort) if abort.is_resumed));

    let stopped_session = restore_compacting(&store, &provider, 1);
    let resuming = stopped_session.resume().unwrap();
    until(|| provider.requests().len() == 4).await;
    let stopped = stopped_session.abort().await;

    assert_eq!(stopped.target, Some(AbortTarget::ActiveCompaction));
    assert_eq!(resuming.await, Ok(ActionEnd::Aborted));
    let blocks = stopped_session.transcript().blocks;
    assert_eq!(kinds(&blocks), ["user", "text", "abort", "resume", "abort"]);
    assert!(matches!(&blocks[2], Block::Abort(abort) if abort.is_resumed));
    assert_eq!(provider.requests().len(), 4);
}

#[tokio::test(flavor = "local")]
async fn a_resume_with_a_pending_switch_to_a_smaller_window_unwinds_then_compacts_once_with_the_model_before_it()
 {
    let question = format!("old question {}", "x".repeat(3_200));
    let provider = ScriptedRuntime::new([
        answer("old answer"),
        partial_then_hang("partial answer"),
        answer("switch summary"),
        answer("continued after the switch"),
    ]);
    let store = MemoryTreeStore::new();
    let config = SessionConfig {
        compaction: only_when_asked(1),
        ..SessionConfig::default()
    };
    let written = start(&provider, Vec::new(), &store, config).await;
    written
        .send(text(&question), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    let running = written.send(text("follow-up"), turn("t2")).unwrap();
    until(|| written.transcript().blocks.len() == 5).await;
    written.abort().await;
    running.await.unwrap();
    let session = restore_compacting(&store, &provider, 1);
    session
        .update_model(ModelSwitch {
            model: small_model(),
            runtime: None,
            apply: ModelSwitchApply::NextTurn,
        })
        .unwrap();

    session.resume().unwrap().await.unwrap();

    // One pass, with the model before the switch; the new model continues.
    let requests = provider.requests();
    let [summary, continuation] = &requests[2..] else {
        panic!("{requests:?}");
    };
    assert!(is_copy(summary) && !is_copy(continuation));
    assert_eq!(summary.model_id, "test-model");
    assert_eq!(continuation.model_id, "small-model");
    assert_eq!(
        continuation.items.as_ref(),
        [summary_item("switch summary"), user_item(RESUME_TEXT)]
    );
    // The unwind kept the stop, the switch's pass summarized what came
    // before it, and the one `resume` block follows the marker.
    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        [
            "user",
            "text",
            "response",
            "user",
            "text",
            "compaction_boundary",
            "abort",
            "compaction_marker",
            "resume",
            "text",
            "response"
        ]
    );
    let (Block::CompactionBoundary(boundary), Block::CompactionMarker(marker)) =
        (&blocks[5], &blocks[7])
    else {
        unreachable!();
    };
    assert_eq!(marker.boundary_id, boundary.id);
    assert!(matches!(&blocks[6], Block::Abort(abort) if abort.is_resumed));
}

#[tokio::test(flavor = "local")]
async fn a_round_whose_usage_with_its_cache_reaches_the_threshold_is_summarized_whole_and_its_tool_never_runs_again()
 {
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![
            event::tool_call("call-1", "count", json!({})),
            // Only the cache write puts the usage over the threshold of 800.
            ProviderEvent::Response(TokenUsage {
                input_tokens: 1,
                output_tokens: 1,
                cache_read_tokens: 0,
                cache_write_tokens: 850,
            }),
        ]),
        answer("tool summary"),
        answer("continued"),
    ]);
    let store = MemoryTreeStore::new();
    let runs = Rc::new(RefCell::new(0));
    let count = tool("count", {
        let runs = runs.clone();
        move |_| {
            *runs.borrow_mut() += 1;
            Box::pin(async { Ok(output("counted")) })
        }
    });
    let compaction = CompactionConfig {
        keep_recent_tokens: 1,
        threshold_percent: Some(80),
    };
    let session = small_session_with(&provider, vec![count], &store, compaction).await;

    session
        .send(text("use the tool"), turn("t1"))
        .unwrap()
        .await
        .unwrap();

    assert_eq!(*runs.borrow(), 1);
    let requests = provider.requests();
    let [_, summary, continuation] = requests.as_slice() else {
        panic!("{requests:?}");
    };
    // The round is summarized whole: the call with its result.
    assert_eq!(
        item_kinds(&summary.items),
        ["user_message", "tool_use", "tool_result", "user_message"]
    );
    assert!(matches!(
        &summary.items[2],
        InferenceItem::ToolResult { output, .. } if *output == texts(&["counted"])
    ));
    assert_eq!(
        continuation.items.as_ref(),
        [summary_item("tool summary"), user_item(RESUME_TEXT)]
    );
    assert_eq!(
        kinds(&session.transcript().blocks),
        [
            "user",
            "tool_call:completed",
            "response",
            "compaction_boundary",
            "compaction_marker",
            "resume",
            "text",
            "response"
        ]
    );
}

#[tokio::test(flavor = "local")]
async fn a_pass_with_nothing_to_summarize_but_the_last_summary_sends_no_request() {
    let provider = ScriptedRuntime::new([answer("first"), answer("summary"), answer("second")]);
    let store = MemoryTreeStore::new();
    let session = small_session(&provider, Vec::new(), &store).await;

    session.compact().unwrap().await.unwrap();

    assert!(provider.requests().is_empty());
    assert!(session.transcript().blocks.is_empty());
    assert_eq!(session.phase(), SessionPhase::Idle);

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
    // The pass keeps the message after the boundary, so its window would
    // hold the last summary alone.
    session.compact().unwrap().await.unwrap();

    assert_eq!(provider.requests().len(), 3);
    assert_eq!(
        kinds(&session.transcript().blocks),
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
}

#[tokio::test(flavor = "local")]
async fn a_second_pass_folds_the_first_summary_in_and_a_restored_session_replays_only_the_last() {
    let provider = ScriptedRuntime::new([
        answer("first answer"),
        answer("summary one"),
        answer("second answer"),
        answer("summary two"),
        answer("third answer"),
        answer("answer after the restore"),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session(&provider, Vec::new(), &store).await;
    let second = "b".repeat(400);
    let third = "c".repeat(2_800);
    session
        .send(long_message(), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    session
        .send(text(&second), turn("t2"))
        .unwrap()
        .await
        .unwrap();

    session
        .send(text(&third), turn("t3"))
        .unwrap()
        .await
        .unwrap();

    // The second window starts at the first boundary: the first summary
    // folds into the second, and nothing before it is summarized again.
    let requests = provider.requests();
    let [.., second_summary, third_request] = requests.as_slice() else {
        panic!("{requests:?}");
    };
    assert!(is_copy(second_summary));
    assert_eq!(
        second_summary.items.as_ref(),
        [
            summary_item("summary one"),
            user_item(&second),
            answer_item("small-model", "second answer"),
            user_item(COMPACTION_SUMMARY_INSTRUCTION),
        ]
    );
    assert_eq!(
        third_request.items.as_ref(),
        [summary_item("summary two"), user_item(&third)]
    );

    // Restored from its checkpoint, the session replays from the last
    // boundary as well.
    let restored_store = store.copy();
    let (restored, _) = restore_session(
        restored_store.checkpoint(&root()).unwrap(),
        &restored_store,
        &provider,
        test_runtime(Vec::new()),
        Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
    );
    restored
        .send(text("after the restore"), turn("t4"))
        .unwrap()
        .await
        .unwrap();

    let requests = provider.requests();
    assert_eq!(
        requests.last().unwrap().items.as_ref(),
        [
            summary_item("summary two"),
            user_item(&third),
            answer_item("small-model", "third answer"),
            user_item("after the restore"),
        ]
    );
}

#[tokio::test(flavor = "local")]
async fn input_that_arrives_during_a_pass_waits_outside_the_summary_for_the_first_request_after_it()
{
    let (first_summary, first_release) =
        gated_turn(vec![event::text("summary one"), event::response(0, 0)]);
    let (second_summary, second_release) =
        gated_turn(vec![event::text("summary two"), event::response(0, 0)]);
    let provider = ScriptedRuntime::new([
        answer("first answer"),
        first_summary,
        answer("second answer"),
        answer("third answer"),
        second_summary,
        answer("fourth answer"),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session(&provider, Vec::new(), &store).await;
    let second = "y".repeat(400);
    let third = "z".repeat(400);
    session
        .send(long_message(), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    let second_turn = session.send(text(&second), turn("t2")).unwrap();
    until(|| provider.requests().len() == 2).await;

    // During the pass before t2's turn, a message waits in the queue and a
    // steer for t2.
    let third_turn = session.send(text(&third), turn("t3")).unwrap();
    session
        .steer(text("mind the tests"), BlockId::try_from("s1").unwrap())
        .unwrap();
    assert_eq!(session.phase(), SessionPhase::Compacting);
    assert_eq!(
        session.queued_messages(),
        [QueuedMessage {
            id: turn("t3"),
            content: text(&third),
        }]
    );
    let steers = session.pending_steers();
    assert_eq!(steers.len(), 1);
    assert_eq!(steers[0].turn_id, turn("t2"));
    let _ = first_release.send(());
    second_turn.await.unwrap();
    third_turn.await.unwrap();

    // The summary holds neither; t2's first request carries the steer, and
    // t3 runs after t2.
    let requests = provider.requests();
    assert_eq!(
        item_kinds(&requests[1].items),
        ["user_message", "assistant_text", "user_message"]
    );
    let steer = InferenceItem::UserSteer {
        content: text("mind the tests"),
    };
    assert_eq!(
        requests[2].items.as_ref(),
        [
            summary_item("summary one"),
            user_item(&second),
            steer.clone()
        ]
    );
    assert_eq!(
        requests[3].items.as_ref(),
        [
            summary_item("summary one"),
            user_item(&second),
            steer,
            answer_item("small-model", "second answer"),
            user_item(&third),
        ]
    );

    // During a `compact` action, a message waits in the queue, and its turn
    // starts from the new summary.
    let compacting = session.compact().unwrap();
    until(|| provider.requests().len() == 5).await;
    let fourth_turn = session.send(text("after the pass"), turn("t4")).unwrap();
    assert_eq!(session.phase(), SessionPhase::Compacting);
    assert_eq!(session.queued_messages().len(), 1);
    let _ = second_release.send(());
    compacting.await.unwrap();
    fourth_turn.await.unwrap();

    assert_eq!(
        provider.requests()[5].items.as_ref(),
        [
            summary_item("summary two"),
            user_item(&third),
            answer_item("small-model", "third answer"),
            user_item("after the pass"),
        ]
    );
}
