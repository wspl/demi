//! Compaction (`compaction.md`): passes before a turn, inside a turn and
//! before a model switch, the session copy that writes the summary, the
//! `compact` action, and the request prefix a provider caches.

use demi_agent_session::{
    TranscriptSnapshot, compaction_refusal, testing::COMPACTION_SUMMARY_INSTRUCTION,
};
use demi_agent_store::{
    media::{BlobStore, ModelView},
    testing::{MemoryBlobs, png},
};
use demi_agent_transcript::{
    COMPACTION_NOTE, RequestView, estimate::block_tokens, testing::RESUME_TEXT,
};
use demi_provider_common::{PromptCache, ProviderFailure, RequestLimits, testing::TokioClock};
use demi_shared_types::{BlockId, MediaSource, QueuedMessage, TokenUsage};

use super::*;

/// A model whose window makes 800 tokens the threshold.
fn small_model() -> ModelSelection {
    let mut model = model_of("stub", "small-model");
    model.model.context_window = 1_000;
    model
}

/// A session whose model has a 1,000-token window.
async fn small_session(
    provider: &ScriptedRuntime,
    tools: Vec<(String, Invoke)>,
    store: &Rc<MemoryTreeStore>,
) -> AgentSession {
    small_session_with(provider, tools, store, CompactionConfig::default()).await
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
            model: Box::new(small_model()),
            runtime: None,
        })
        .unwrap();
    session
}

/// Compaction that runs only when asked, so the history written with it can
/// grow over any threshold.
fn only_when_asked() -> CompactionConfig {
    CompactionConfig { automatic: false }
}

/// The node `root` restored from a copy of `store`, compacting at 80% of its
/// model's window: a history written without compaction meets a threshold it
/// is over.
fn restore_compacting(store: &MemoryTreeStore, provider: &ScriptedRuntime) -> AgentSession {
    let copy = store.copy();
    let checkpoint = copy.checkpoint(&root()).unwrap();
    let config = SessionConfig {
        compaction: CompactionConfig::default(),
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
fn long_message() -> Vec<demi_shared_types::UserContentBlock> {
    text(&"x".repeat(3_000))
}

/// Whether `request` is a summary request: its last user message ends with
/// the summary instruction, which no request of the session carries.
fn is_copy(request: &demi_provider_common::InferenceRequest) -> bool {
    request.items.iter().any(|item| match item {
        InferenceItem::UserMessage { content } | InferenceItem::UserSteer { content } => {
            content.last() == Some(&UserPart::Text(COMPACTION_SUMMARY_INSTRUCTION.to_owned()))
        }
        _ => false,
    })
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
        content: sent_text(message),
    }
}

/// The user's `message` with the summary instruction as one more text
/// block, as a summary request ends a history that ends with the message.
fn instructed_item(message: &str) -> InferenceItem {
    let mut content = sent_text(message);
    content.push(UserPart::Text(COMPACTION_SUMMARY_INSTRUCTION.to_owned()));
    InferenceItem::UserMessage { content }
}

/// The summary of a `compaction_boundary` as the model receives it.
fn summary_item(summary: &str) -> InferenceItem {
    user_item(&format!("<summary>\n{summary}\n</summary>"))
}

/// What the model receives of a `compaction_boundary` that kept the user's
/// messages `kept`, oldest first, and holds `summary`: the note, each
/// message, then the summary.
fn boundary_items(kept: &[&str], summary: &str) -> Vec<InferenceItem> {
    let mut items = vec![user_item(COMPACTION_NOTE)];
    items.extend(kept.iter().map(|message| user_item(message)));
    items.push(summary_item(summary));
    items
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
    let changes = Rc::new(RefCell::new(Vec::new()));
    let _listener = session.subscribe({
        let changes = changes.clone();
        move |event| match event {
            SessionEvent::PhaseChanged { phase } => changes.borrow_mut().push(format!("{phase:?}")),
            SessionEvent::WaitingChanged { waited_ms } => changes
                .borrow_mut()
                .push(if waited_ms.is_some() { "waits" } else { "waits no more" }.to_owned()),
            _ => {}
        }
    });
    session
        .send(long_message(), turn("t1"))
        .unwrap()
        .await
        .unwrap();

    changes.borrow_mut().clear();
    session
        .send(text(&"y".repeat(400)), turn("t2"))
        .unwrap()
        .await
        .unwrap();

    let requests = provider.requests();
    let [first, summary, second] = requests.as_slice() else {
        panic!("{requests:?}");
    };
    // The copy's request is the request the second message would have
    // made, the first as the session sent it, its answer and the message,
    // with the instruction as one more text block of that message; same
    // system prompt, tools and model.
    assert!(is_copy(summary) && !is_copy(second));
    // It carries the session's id, which keeps it with the session's
    // requests at the vendor.
    assert_eq!(summary.session_id, first.session_id);
    let mut expected = first.items.to_vec();
    expected.extend([
        answer_item("small-model", "first answer"),
        instructed_item(&"y".repeat(400)),
    ]);
    assert_eq!(summary.items.as_ref(), expected.as_slice());
    assert_eq!(summary.system_prompt, second.system_prompt);
    assert_eq!(summary.model_id, "small-model");
    // The boundary stands where that request's content ends; the answer to
    // it and the next message are kept, and the marker ends them.
    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        [
            "user",
            "compaction_boundary",
            "text",
            "response",
            "user",
            "compaction_marker",
            "text",
            "response"
        ]
    );
    let Block::CompactionBoundary(boundary) = &blocks[1] else {
        unreachable!();
    };
    assert_eq!(boundary.summary, "summary of the first turn");
    let Block::CompactionMarker(marker) = &blocks[5] else {
        unreachable!();
    };
    assert_eq!(marker.boundary_id, boundary.id);
    let summarized = ModelView::of(0, &blocks[..1], &HeldMedia::default()).unwrap();
    let model = small_model().model;
    let request = RequestView::new(&summarized, &model, RequestLimits::default(), &[]);
    assert_eq!(marker.compacted_tokens, block_tokens(&blocks[0], &request));
    // The boundary keeps the user's message beside its summary.
    assert_eq!(
        second.items.as_ref(),
        [
            // The message, cut to the 100 tokens of the user's words a
            // 1,000-token window keeps.
            boundary_items(&[&"x".repeat(400)], "summary of the first turn"),
            vec![
                answer_item("small-model", "first answer"),
                user_item(&"y".repeat(400)),
            ],
        ]
        .concat()
    );
    // The turn waits for the model from its start; the page shows the pass
    // instead of the wait, which starts again at the pass's end
    // (`runtime.md` § Waiting for the model).
    assert_eq!(
        *changes.borrow(),
        ["Running", "waits", "Compacting", "waits no more", "Running", "waits", "waits no more", "Idle"]
    );
    // The copy saved nothing and its runtime was closed.
    assert!(store.saves().iter().all(|(node, _)| node == &root()));
    assert_eq!(provider.closes(), 1);
}

// One session and four scripted requests: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_limit_on_the_models_window_lowers_the_token_threshold_from_the_next_check() {
    let provider = ScriptedRuntime::new([
        answer("first answer"),
        answer("second answer"),
        answer("summary of the first turns"),
        answer("third answer"),
    ]);
    let store = MemoryTreeStore::new();
    let runtime = test_runtime(Vec::new());
    let window = runtime.window_in_use.clone();
    let session = start_on(&provider, runtime, &store, SessionConfig::default()).await;
    session
        .update_model(ModelSwitch {
            model: Box::new(small_model()),
            runtime: None,
        })
        .unwrap();
    session
        .send(text(&"x".repeat(1_600)), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    // About 430 tokens: under 500, the threshold of the model's own window,
    // half of it.
    session.send(text("second"), turn("t2")).unwrap().await.unwrap();

    // The user limits the model's window to 600 tokens; the next turn's
    // check compacts at 300.
    window.set(Some(600));
    session.send(text("third"), turn("t3")).unwrap().await.unwrap();

    let copies: Vec<bool> = provider.requests().iter().map(is_copy).collect();
    assert_eq!(copies, [false, false, true, false]);
    let kinds = kinds(&session.transcript().blocks);
    assert!(
        kinds.iter().any(|kind| kind == "compaction_boundary"),
        "{kinds:?}"
    );
}

#[tokio::test(flavor = "local")]
async fn a_request_repeats_the_one_before_as_its_prefix_a_pass_restarts_it_at_its_summary_and_a_restored_session_asks_the_same()
 {
    let provider = ScriptedRuntime::new([
        answer("answer A"),
        answer("answer B"),
        answer("summary of A"),
        answer("answer C"),
        answer("answer D"),
        answer("answer D again"),
    ]);
    let store = MemoryTreeStore::new();
    let (note, _) = counted("note", "noted");
    let session =
        small_session_with(&provider, vec![note.clone()], &store, only_when_asked()).await;
    let question_b = "b".repeat(480);
    session.send(text("A"), turn("A")).unwrap().await.unwrap();
    session
        .send(text(&question_b), turn("B"))
        .unwrap()
        .await
        .unwrap();
    session.compact().unwrap().await.unwrap();
    session.send(text("C"), turn("C")).unwrap().await.unwrap();
    let before_d = store.copy();
    session.send(text("D"), turn("D")).unwrap().await.unwrap();
    // The same history in another session: other block ids, another clock.
    let (restored, _) = restore_session(
        before_d.checkpoint(&root()).unwrap(),
        &before_d,
        &provider,
        test_runtime(vec![note]),
        Arc::new(FixedClock("2026-09-26T12:00:00Z".parse().unwrap())),
    );
    restored.send(text("D"), turn("D")).unwrap().await.unwrap();

    let requests = provider.requests();
    let [a, b, summary, c, d, d_again] = requests.as_slice() else {
        panic!("{requests:?}");
    };
    // The summary request is the whole history, b and its answer, with the
    // instruction after it as a message of its own, since the history ends
    // with an answer.
    assert_eq!(
        summary.items.as_ref(),
        [
            b.items.to_vec(),
            vec![
                answer_item("small-model", "answer B"),
                user_item(COMPACTION_SUMMARY_INSTRUCTION)
            ]
        ]
        .concat()
    );
    // Each request repeats the one before and its answer, with the same
    // system prompt and tools.
    assert_eq!(
        b.items.as_ref(),
        [
            a.items.to_vec(),
            vec![
                answer_item("small-model", "answer A"),
                user_item(&question_b)
            ]
        ]
        .concat()
    );
    for request in [b, c, d] {
        assert_eq!(request.system_prompt, a.system_prompt);
        assert_eq!(request.tools, a.tools);
    }
    // A pass starts the prefix again at its boundary, which stands at the
    // end of a, the latest answered request that leaves the recent work
    // after it: the 166 estimated tokens after its end, the question B
    // among them, reach the 150 a 1,000-token window keeps, and the 23
    // after b do not. The boundary keeps the message A, with what came
    // after a's end as it was written, and the next request repeats that.
    assert_eq!(
        c.items.as_ref(),
        [
            boundary_items(&["A"], "summary of A"),
            vec![
                answer_item("small-model", "answer A"),
                user_item(&question_b),
                answer_item("small-model", "answer B"),
                user_item("C"),
            ],
        ]
        .concat()
    );
    assert_eq!(
        d.items.as_ref(),
        [
            c.items.to_vec(),
            vec![answer_item("small-model", "answer C"), user_item("D")]
        ]
        .concat()
    );
    // Each request says how many of its items the latest answered request
    // carried: none before the first answer, and none after a pass until the
    // first answer after it.
    assert_eq!(
        [a, b, c, d].map(|request| request.prompt_cache),
        [0, a.items.len(), 0, c.items.len()]
            .map(|answered_items| PromptCache::Session { answered_items })
    );
    // What a request holds comes from the history alone.
    assert_eq!(
        (
            d_again.items.as_ref(),
            &d_again.system_prompt,
            &d_again.tools,
            &d_again.model_id
        ),
        (d.items.as_ref(), &d.system_prompt, &d.tools, &d.model_id)
    );
}

#[tokio::test(flavor = "local")]
async fn a_summary_request_that_exceeds_the_context_is_retried_with_the_window_then_half_of_it() {
    let provider = ScriptedRuntime::new([
        answer("one"),
        answer("two"),
        too_long(),
        too_long(),
        answer("summary of the first half"),
        answer("done"),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session(&provider, Vec::new(), &store).await;
    session
        .send(text(&"a".repeat(600)), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    session
        .send(text(&"b".repeat(600)), turn("t2"))
        .unwrap()
        .await
        .unwrap();

    session
        .send(text(&"c".repeat(800)), turn("t3"))
        .unwrap()
        .await
        .unwrap();

    // The first summary request held the whole history, five items, the
    // instruction joined to the third message; the second the window alone,
    // what the second request carried, three items, the instruction joined
    // to its second message; the third the first half of that window, two
    // blocks, and the instruction after the answer.
    assert_eq!(summary_sizes(&provider), [5, 3, 3]);
    let blocks = session.transcript().blocks;
    let boundaries = blocks
        .iter()
        .filter(|block| matches!(block, Block::CompactionBoundary(_)))
        .count();
    assert_eq!(boundaries, 1);
    assert!(
        matches!(&blocks[2], Block::CompactionBoundary(_)),
        "{:?}",
        kinds(&blocks)
    );
}

#[tokio::test(flavor = "local")]
async fn a_blank_summary_compacts_nothing_and_a_failed_one_fails_the_turn_with_its_failure_record() {
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
    // The failed summary request is the turn's failure record, as a failed
    // request of the turn's own would be, and the turn it ended is
    // unfinished.
    assert_eq!(
        kinds(&blocks),
        [
            "user", "text", "response", "user", "text", "response", "user", "error"
        ]
    );
    let Some(Block::Error(error)) = blocks.last() else {
        unreachable!("the kinds end with an error");
    };
    assert!(!error.outside_turn);
}

/// A request refused for an expired key, which is never retried.
fn expired_key() -> Turn {
    Turn::Events(vec![event::error(
        "the key expired",
        Some(ErrorCode::AuthExpired),
    )])
}

// One session and two scripted requests: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_failed_compact_after_a_finished_answer_leaves_a_failure_record_that_ended_no_turn() {
    let provider = ScriptedRuntime::new([answer("first"), expired_key()]);
    let store = MemoryTreeStore::new();
    let session = small_session_with(&provider, Vec::new(), &store, only_when_asked()).await;
    session
        .send(long_message(), turn("t1"))
        .unwrap()
        .await
        .unwrap();

    let failed = session.compact().unwrap().await;

    assert_eq!(failed.unwrap_err().code.as_deref(), Some("auth_expired"));
    let blocks = session.transcript().blocks;
    assert_eq!(kinds(&blocks), ["user", "text", "response", "error"]);
    let Some(Block::Error(error)) = blocks.last() else {
        unreachable!("the kinds end with an error");
    };
    assert!(error.outside_turn, "{error:?}");
}

// One session and two scripted requests: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_compact_stopped_after_a_finished_answer_leaves_no_stop_to_continue() {
    let provider = ScriptedRuntime::new([answer("first"), Turn::pending()]);
    let store = MemoryTreeStore::new();
    let session = small_session_with(&provider, Vec::new(), &store, only_when_asked()).await;
    session
        .send(long_message(), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    let compacting = session.compact().unwrap();
    until(|| provider.requests().len() == 2).await;

    let stopped = session.abort().await;

    assert_eq!(stopped.target, Some(AbortTarget::ActiveCompaction));
    assert_eq!(compacting.await, Ok(ActionEnd::Aborted));
    // The answer had finished: no `abort` block offers Continue on it.
    assert_eq!(
        kinds(&session.transcript().blocks),
        ["user", "text", "response"]
    );
}

// One session and five scripted requests: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_compact_stopped_after_a_finished_turn_holds_neither_a_later_message_nor_a_report() {
    let provider = ScriptedRuntime::new([
        Turn::Events(background_call("17", None, "Build")),
        answer("building"),
        Turn::pending(),
        answer("read the news"),
        answer("checked the build"),
    ]);
    let store = MemoryTreeStore::new();
    let config = SessionConfig {
        compaction: only_when_asked(),
        ..SessionConfig::default()
    };
    let runtime = test_runtime(Vec::new());
    let commands = runtime.commands.clone();
    let runtime = TestRuntime {
        tools: vec![background_tool(&commands)],
        ..runtime
    };
    let session = start_at(
        &provider,
        runtime,
        &store,
        config,
        Arc::new(TokioClock::new(Timestamp::UNIX_EPOCH)),
    )
    .await;
    session
        .update_model(ModelSwitch {
            model: Box::new(small_model()),
            runtime: None,
        })
        .unwrap();
    session
        .send(long_message(), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    let compacting = session.compact().unwrap();
    until(|| provider.requests().len() == 3).await;
    session.abort().await;
    assert_eq!(compacting.await, Ok(ActionEnd::Aborted));
    // The stop wrote nothing: no marker.
    assert_eq!(
        kinds(&session.transcript().blocks),
        ["user", "tool_call:completed", "response", "text", "response"]
    );

    // A message that comes after the stop wakes the session as usual.
    session
        .accept_agent_message(agent_message("news"))
        .await
        .unwrap();
    session.settled().await;
    assert_eq!(
        kinds(&session.transcript().blocks[5..]),
        ["agent_message", "text", "response"]
    );
    // So does the report of the command the turn left running, once it
    // ends.
    commands.end("17");
    until(|| provider.requests().len() == 5).await;
    session.settled().await;
    assert_eq!(
        kinds(&session.transcript().blocks[8..]),
        ["wakeup", "text", "response"]
    );
}

// One session and four scripted requests: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_failed_turn_compacted_afterwards_is_resumed_after_the_summary_and_keeps_the_divider() {
    let provider = ScriptedRuntime::new([
        answer("first"),
        expired_key(),
        answer("summary"),
        answer("second"),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session_with(&provider, Vec::new(), &store, only_when_asked()).await;
    session
        .send(long_message(), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    session
        .send(text("again"), turn("t2"))
        .unwrap()
        .await
        .unwrap_err();
    session.compact().unwrap().await.unwrap();
    assert_eq!(
        kinds(&session.transcript().blocks),
        [
            "user",
            "compaction_boundary",
            "text",
            "response",
            "user",
            "error",
            "compaction_marker"
        ]
    );

    session.resume(None).unwrap().await.unwrap();

    // The failure left nothing but its record, so the turn reruns from its
    // message, after the summary; the compaction stays where it was.
    let requests = provider.requests();
    assert_eq!(
        requests[3].items.as_ref(),
        [
            boundary_items(&[&"x".repeat(400)], "summary"),
            vec![
                answer_item("small-model", "first"),
                user_item("again"),
            ],
        ]
        .concat()
    );
    assert_eq!(
        kinds(&session.transcript().blocks),
        [
            "user",
            "compaction_boundary",
            "text",
            "response",
            "user",
            "compaction_marker",
            "text",
            "response"
        ]
    );
}

// One session and one scripted request: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn the_usage_and_the_compact_gate_count_against_the_limit_the_user_set() {
    let provider = ScriptedRuntime::new([Turn::Events(vec![
        event::text("first"),
        event::response(400, 0),
    ])]);
    let store = MemoryTreeStore::new();
    let runtime = test_runtime(Vec::new());
    let window = runtime.window_in_use.clone();
    let session = start_on(&provider, runtime, &store, SessionConfig::default()).await;
    session
        .update_model(ModelSwitch {
            model: Box::new(small_model()),
            runtime: None,
        })
        .unwrap();
    session.send(text("first"), turn("t1")).unwrap().await.unwrap();

    let full = session.context_usage().await.unwrap();
    assert_eq!(
        (full.tokens, full.window, full.compact_from),
        (400, Some(1_000), Some(500))
    );
    assert_eq!(
        compaction_refusal(&full, false).as_deref(),
        Some("Available from 50% context (now 40%)")
    );

    // The user limits the model's window to 600 tokens.
    window.set(Some(600));
    let limited = session.context_usage().await.unwrap();
    assert_eq!(
        (limited.tokens, limited.window, limited.compact_from),
        (400, Some(600), Some(300))
    );
    assert_eq!(compaction_refusal(&limited, false), None);
    assert_eq!(session.held_context_usage().await, Some(limited));
}

/// A message with `image`, which the store's namespace holds.
fn image_message(image: &BlobRef) -> Vec<UserContentBlock> {
    vec![
        UserContentBlock::Text {
            text: "What is on the screen?".into(),
        },
        UserContentBlock::Image {
            source: MediaSource::Ref {
                r#ref: image.clone(),
                media_type: "image/png".into(),
                width: None,
                height: None,
            },
        },
    ]
}

/// The node `root` restored from a copy of `store`, which holds no media's
/// bytes yet.
fn restored(store: &MemoryTreeStore, provider: &ScriptedRuntime) -> AgentSession {
    let copy = store.copy();
    let checkpoint = copy.checkpoint(&root()).unwrap();
    let (session, _) = restore_session(
        checkpoint,
        &copy,
        provider,
        test_runtime(Vec::new()),
        Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
    );
    session
}

// Two restores and one blob read from memory: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_restored_session_tells_its_usage_without_a_blob_while_its_media_lie_before_the_anchor() {
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![event::text("A login form."), event::response(5_000, 10)]),
        expired_key(),
    ]);
    let blobs = MemoryBlobs::new();
    let store = MemoryTreeStore::with_blobs(blobs.clone());
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    session
        .update_model(ModelSwitch {
            model: Box::new(model_reading("stub", "model-a", &[FileExtension::Png])),
            runtime: None,
        })
        .unwrap();
    let image = blobs.put(png(4, 3, 1)).await.unwrap();
    session
        .send(image_message(&image), turn("t1"))
        .unwrap()
        .await
        .unwrap();

    // The image went out in the request the latest response measured: the
    // estimate is that usage, and needs no byte of it.
    assert_eq!(
        restored(&store, &provider)
            .held_context_usage()
            .await
            .map(|usage| usage.tokens),
        Some(5_010)
    );

    // The next message's image was never answered, so it weighs, and only
    // its bytes can say how much.
    let failed = session.send(image_message(&image), turn("t2")).unwrap().await;
    assert!(failed.is_err());
    let after_failure = restored(&store, &provider);
    assert_eq!(after_failure.held_context_usage().await, None);
    let read = after_failure.context_usage().await.unwrap();
    assert!(read.tokens >= 5_010 + 1_600, "{read:?}");
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

/// A summary answer that calls `note`, with `text` beside the call when
/// there is some.
fn calling_answer(id: &str, text: Option<&str>) -> Turn {
    let mut events: Vec<ProviderEvent> = text.map(event::text).into_iter().collect();
    events.push(event::tool_call(id, "note", json!({})));
    events.push(event::response(0, 0));
    Turn::Events(events)
}

// Three sessions and eleven scripted requests, the retry's wait on a paused
// clock: a few milliseconds.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_tool_the_model_calls_during_a_summary_never_runs() {
    // Beside text, the call is left: the text is the summary.
    let provider = ScriptedRuntime::new([
        answer("first"),
        calling_answer("beside", Some("summary beside a call")),
        answer("second"),
    ]);
    let store = MemoryTreeStore::new();
    let (note, runs) = counted("note", "noted");
    let session = small_session(&provider, vec![note.clone()], &store).await;
    session.send(long_message(), turn("t1")).unwrap().await.unwrap();
    session.send(text(&"y".repeat(400)), turn("t2")).unwrap().await.unwrap();
    assert_eq!(runs.get(), 0);
    let blocks = session.transcript().blocks;
    assert!(!blocks.iter().any(|block| matches!(block, Block::ToolCall(_))));
    let Block::CompactionBoundary(boundary) = &blocks[1] else {
        panic!("{:?}", kinds(&blocks));
    };
    assert_eq!(boundary.summary, "summary beside a call");

    // A call alone is a failed summary request: it is asked again, and the
    // client hears of the retry.
    let provider = ScriptedRuntime::new([
        answer("first"),
        calling_answer("alone", None),
        answer("summary asked again"),
        answer("second"),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session(&provider, vec![note.clone()], &store).await;
    let retries = Rc::new(Cell::new(0));
    let _listener = session.subscribe({
        let retries = retries.clone();
        move |event| {
            if let SessionEvent::RetryScheduled { .. } = event {
                retries.set(retries.get() + 1);
            }
        }
    });
    session.send(long_message(), turn("t1")).unwrap().await.unwrap();
    session.send(text(&"y".repeat(400)), turn("t2")).unwrap().await.unwrap();
    assert_eq!(runs.get(), 0);
    assert_eq!(retries.get(), 1);
    let requests = provider.requests();
    assert_eq!(requests[2].items, requests[1].items);
    let blocks = session.transcript().blocks;
    let Block::CompactionBoundary(boundary) = &blocks[1] else {
        panic!("{:?}", kinds(&blocks));
    };
    assert_eq!(boundary.summary, "summary asked again");

    // A second answer of a call alone fails the pass, and with it the turn.
    let provider = ScriptedRuntime::new([
        answer("first"),
        calling_answer("alone", None),
        calling_answer("again", None),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session(&provider, vec![note], &store).await;
    session.send(long_message(), turn("t1")).unwrap().await.unwrap();
    let failed = session.send(text(&"y".repeat(400)), turn("t2")).unwrap().await;
    assert!(failed.is_err());
    assert_eq!(runs.get(), 0);
    assert_eq!(
        kinds(&session.transcript().blocks),
        ["user", "text", "response", "user", "error"]
    );
}

#[tokio::test(flavor = "local")]
async fn a_turn_whose_usage_keeps_reaching_the_threshold_compacts_at_most_three_times() {
    // Every response reports 900 tokens, over the threshold of 500: each
    // round of tools is followed by a pass, until the third. The user's
    // messages are short, so each pass frees what the answers held.
    let round = |call: &str| {
        Turn::Events(vec![
            event::tool_call(call, "work", json!({})),
            event::response(900, 0),
        ])
    };
    let provider = ScriptedRuntime::new([
        answer(&"f".repeat(1_600)),
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
    session.send(text("start"), turn("t1")).unwrap().await.unwrap();

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

// One session and four scripted requests: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_final_answer_over_the_threshold_ends_its_turn_and_the_next_send_compacts_first() {
    // The answer requests no tool and reports 900 tokens, over the
    // threshold of 500: the turn ends as answered, with no pass and no
    // request asking the model to go on and write it again.
    let provider = ScriptedRuntime::new([
        answer("first answer"),
        Turn::Events(vec![event::text("the report"), event::response(900, 0)]),
        answer("summary"),
        answer("welcome"),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session(&provider, Vec::new(), &store).await;
    session
        .send(text(&"x".repeat(1_200)), turn("t1"))
        .unwrap()
        .await
        .unwrap();

    session
        .send(text("write the report"), turn("t2"))
        .unwrap()
        .await
        .unwrap();

    assert_eq!(provider.requests().len(), 2);
    assert_eq!(
        kinds(&session.transcript().blocks),
        ["user", "text", "response", "user", "text", "response"]
    );

    // The next send compacts before its turn, from the answer's usage, and
    // its request starts from the summary.
    session.send(text("thanks"), turn("t3")).unwrap().await.unwrap();

    let requests = provider.requests();
    let copies: Vec<bool> = requests.iter().map(is_copy).collect();
    assert_eq!(copies, [false, false, true, false]);
    let next = &requests[3];
    assert_eq!(next.items.first(), Some(&user_item(COMPACTION_NOTE)));
    assert!(next.items.contains(&summary_item("summary")));
    assert!(
        next.items.ends_with(&[user_item("thanks")]),
        "{:#?}",
        next.items
    );
    assert!(!kinds(&session.transcript().blocks).iter().any(|kind| kind == "resume"));
}

#[tokio::test(flavor = "local")]
async fn a_switch_to_a_smaller_window_compacts_with_the_model_before_it() {
    let provider = ScriptedRuntime::new([
        answer(&"f".repeat(1_500)),
        answer(&"s".repeat(1_500)),
        answer("summary by the large model"),
        answer("third"),
        answer("fourth"),
    ]);
    let store = MemoryTreeStore::new();
    let config = SessionConfig {
        compaction: CompactionConfig::default(),
        ..SessionConfig::default()
    };
    let session = start(&provider, Vec::new(), &store, config).await;
    session.send(text("a"), turn("t1")).unwrap().await.unwrap();
    session.send(text("b"), turn("t2")).unwrap().await.unwrap();
    session
        .update_model(ModelSwitch {
            model: Box::new(small_model()),
            runtime: None,
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
            model: Box::new(test_model()),
            runtime: None,
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

/// An answer of 375 estimated tokens.
fn long_answer() -> Turn {
    answer(&"s".repeat(1_500))
}

/// A session with two short messages, answered by [`long_answer`]s that
/// the provider plays first, and a recorded switch to the 1,000-token
/// window, which compacts before it lands.
async fn switching_to_a_smaller_window(provider: &ScriptedRuntime, store: &Rc<MemoryTreeStore>) -> AgentSession {
    let config = SessionConfig {
        compaction: CompactionConfig::default(),
        ..SessionConfig::default()
    };
    let session = start(provider, Vec::new(), store, config).await;
    for (message, id) in [("a", "t1"), ("b", "t2")] {
        session.send(text(message), turn(id)).unwrap().await.unwrap();
    }
    session
        .update_model(ModelSwitch {
            model: Box::new(small_model()),
            runtime: None,
        })
        .unwrap();
    session
}

// One session and five scripted requests: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_send_whose_switch_fails_to_compact_keeps_its_message_in_a_turn_that_resume_finishes() {
    let provider = ScriptedRuntime::new([
        long_answer(),
        long_answer(),
        expired_key(),
        answer("summary by the large model"),
        answer("third"),
    ]);
    let store = MemoryTreeStore::new();
    let session = switching_to_a_smaller_window(&provider, &store).await;

    let failed = session.send(text("short"), turn("t3")).unwrap().await;

    assert_eq!(failed.unwrap_err().code.as_deref(), Some("auth_expired"));
    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        ["user", "text", "response", "user", "text", "response", "user", "error"]
    );
    let Some(Block::Error(error)) = blocks.last() else {
        unreachable!("the kinds end with an error");
    };
    assert!(!error.outside_turn, "{error:?}");

    session.resume(None).unwrap().await.unwrap();

    // Resume lands the switch again, and the message reaches the new model.
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
            ("test-model", true),
            ("small-model", false)
        ]
    );
    assert_eq!(requests[4].items.last(), Some(&user_item("short")));
}

// One session and three scripted requests: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_send_stopped_while_its_switch_compacts_keeps_its_message_before_the_stop() {
    let provider = ScriptedRuntime::new([long_answer(), long_answer(), Turn::pending()]);
    let store = MemoryTreeStore::new();
    let session = switching_to_a_smaller_window(&provider, &store).await;
    let running = session.send(text("short"), turn("t3")).unwrap();
    until(|| provider.requests().len() == 3).await;

    let stopped = session.abort().await;

    assert_eq!(stopped.target, Some(AbortTarget::ActiveCompaction));
    assert_eq!(running.await, Ok(ActionEnd::Aborted));
    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        ["user", "text", "response", "user", "text", "response", "user", "abort"]
    );
    assert!(matches!(&blocks[6], Block::User(user) if user.turn_id == turn("t3")));
}

#[tokio::test(flavor = "local")]
async fn a_switch_inside_a_turn_to_a_smaller_window_compacts_with_the_model_before_it() {
    let provider = ScriptedRuntime::new([
        answer(&"f".repeat(3_600)),
        Turn::Events(vec![
            event::tool_call("call-1", "slow", json!({})),
            event::response(0, 0),
        ]),
        answer("summary by the large model"),
        answer("continued on the small model"),
    ]);
    let store = MemoryTreeStore::new();
    let (slow, releases, started) = gated_tool("slow");
    let config = SessionConfig {
        compaction: CompactionConfig::default(),
        ..SessionConfig::default()
    };
    let session = start(&provider, vec![slow], &store, config).await;
    session.send(text("a"), turn("t1")).unwrap().await.unwrap();
    let running = session.send(text(&"b".repeat(480)), turn("t2")).unwrap();
    started.await.unwrap();

    session
        .update_model(ModelSwitch {
            model: Box::new(small_model()),
            runtime: None,
        })
        .unwrap();
    let _ = releases.borrow_mut().remove(0).send(());
    running.await.unwrap();

    // At the boundary after the tool, the model before the switch
    // summarized what the smaller window cannot hold; the same turn then
    // went on with the new model, after a `resume`.
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
            ("small-model", false)
        ]
    );
    assert_eq!(
        kinds(&session.transcript().blocks),
        [
            "user",
            "text",
            "response",
            "user",
            "compaction_boundary",
            "tool_call:completed",
            "response",
            "compaction_marker",
            "resume",
            "text",
            "response"
        ]
    );
    let continuation = &requests[3];
    assert_eq!(
        item_kinds(&continuation.items),
        [
            "user_message",
            "user_message",
            "user_message",
            "user_message",
            "tool_use",
            "tool_result",
            "user_message"
        ]
    );
    assert_eq!(
        continuation.items[..4],
        boundary_items(&["a", &"b".repeat(480)], "summary by the large model")
    );
    assert_eq!(continuation.items.last(), Some(&user_item(RESUME_TEXT)));
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
        compaction: CompactionConfig::default(),
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
    session
        .accept_agent_message(agent_message("m1"))
        .await
        .unwrap();
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
async fn a_summary_that_exceeds_the_context_down_to_one_block_fails_the_action_with_its_failure_record()
{
    let provider = ScriptedRuntime::new([
        answer("old answer"),
        answer("recent answer"),
        too_long(),
        too_long(),
        too_long(),
        too_long(),
        answer("recovered"),
        answer("summary"),
        answer("fourth answer"),
        too_long(),
        too_long(),
        too_long(),
        too_long(),
        too_long(),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session_with(&provider, Vec::new(), &store, only_when_asked()).await;
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

    // The whole history, then what the second request carried, four
    // blocks, then their first two, then the first alone, each with the
    // instruction joined to a message that ends it: the overflow fails the
    // action, reported once, and the history gains only its failure record.
    let failure = failed.unwrap_err();
    assert_eq!(failure.code.as_deref(), Some("context_length_exceeded"));
    assert_eq!(*errors.borrow(), [*failure]);
    assert_eq!(summary_sizes(&provider), [5, 3, 3, 1]);
    assert_failure_recorded(&session, &before);
    // The session goes on.
    session
        .send(text("recover"), turn("t3"))
        .unwrap()
        .await
        .unwrap();

    // After a boundary, the window shrinks down to the boundary and the
    // block after it, the kept answer, and then fails the same way. The
    // boundary goes after the first message, the end of the latest request
    // that leaves 150 estimated tokens of recent work after it.
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
    // The boundary replays as three messages: the note, the first message
    // it kept, and the summary.
    // The pass between asked once, with the whole history, seven items.
    assert_eq!(summary_sizes(&provider), [5, 3, 3, 1, 7, 11, 9, 7, 5, 5]);
    assert_failure_recorded(&session, &before);
}

/// The history is `before` with one failure record after it, whose code says
/// the summary request exceeded the context.
fn assert_failure_recorded(session: &AgentSession, before: &TranscriptSnapshot) {
    let blocks = session.transcript().blocks;
    assert_eq!(blocks[..blocks.len() - 1], before.blocks[..]);
    let Some(Block::Error(error)) = blocks.last() else {
        panic!("no failure record: {:?}", kinds(&blocks));
    };
    assert_eq!(error.code.as_deref(), Some("context_length_exceeded"));
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
    let written = small_session_with(&provider, Vec::new(), &store, only_when_asked()).await;
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
    let session = restore_compacting(&store, &provider);

    session.retry().unwrap().await.unwrap();

    // Rewound to its message, the history is over the threshold: the pass
    // asks with the rerun's request and the instruction joined to its
    // message, and summarizes what the first request carried before the
    // rerun, whose request starts with the summary and that request's
    // answer.
    let requests = provider.requests();
    let [summary, rerun] = &requests[2..] else {
        panic!("{requests:?}");
    };
    assert_eq!(
        summary.items.as_ref(),
        [
            user_item(&question),
            answer_item("small-model", "old answer"),
            instructed_item("retry this"),
        ]
    );
    assert_eq!(
        rerun.items.as_ref(),
        [
            boundary_items(&[&question[..400]], "retry summary"),
            vec![
                answer_item("small-model", "old answer"),
                user_item("retry this"),
            ],
        ]
        .concat()
    );
    assert_eq!(rerun.turn_id, "t2");
    assert_eq!(
        kinds(&session.transcript().blocks),
        [
            "user",
            "compaction_boundary",
            "text",
            "response",
            "user",
            "compaction_marker",
            "text",
            "response"
        ]
    );

    let stopped_session = restore_compacting(&store, &provider);
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
async fn a_resume_over_the_threshold_keeps_the_stopped_message_after_the_summary_and_a_stop_during_that_pass_leaves_no_boundary()
 {
    let old_answer = format!("old answer {}", "a".repeat(1_200));
    let question = format!("question {}", "q".repeat(2_400));
    let partial = format!("partial answer {}", "y".repeat(300));
    let provider = ScriptedRuntime::new([
        answer(&old_answer),
        answer("summary one"),
        partial_then_hang(&partial),
        // The first restored session: the summary, then the continuation.
        answer("resume summary"),
        answer("continued after the summary"),
        // The second: a summary that ends only when it is stopped.
        Turn::pending(),
    ]);
    let store = MemoryTreeStore::new();
    let written = small_session_with(&provider, Vec::new(), &store, only_when_asked()).await;
    written
        .send(text("old question"), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    written.compact().unwrap().await.unwrap();
    let running = written.send(text(&question), turn("t2")).unwrap();
    until(|| written.transcript().blocks.len() == 7).await;
    written.abort().await;
    running.await.unwrap();
    let session = restore_compacting(&store, &provider);

    session.resume(None).unwrap().await.unwrap();

    // No request was answered since the last pass, so the window ends
    // before the stopped message: the pass summarizes the summary and the
    // answer kept with it, and the model reads the message as the user
    // wrote it, what it wrote before the stop and the resume after the new
    // summary.
    let requests = provider.requests();
    let [summary, continuation] = &requests[3..] else {
        panic!("{requests:?}");
    };
    // The summary request is the continuation's, with the instruction
    // joined to the resume; the first boundary's kept message, the old
    // question, stays kept.
    assert_eq!(
        summary.items.as_ref(),
        [
            boundary_items(&["old question"], "summary one"),
            vec![
                answer_item("small-model", &old_answer),
                user_item(&question),
                answer_item("small-model", &partial),
                instructed_item(RESUME_TEXT),
            ],
        ]
        .concat()
    );
    assert_eq!(
        continuation.items.as_ref(),
        [
            boundary_items(&["old question"], "resume summary"),
            vec![
                user_item(&question),
                answer_item("small-model", &partial),
                user_item(RESUME_TEXT),
            ],
        ]
        .concat()
    );
    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        [
            "user",
            "compaction_boundary",
            "text",
            "response",
            "compaction_marker",
            "compaction_boundary",
            "user",
            "text",
            "abort",
            "resume",
            "compaction_marker",
            "text",
            "response"
        ]
    );
    assert!(matches!(&blocks[8], Block::Abort(abort) if abort.is_resumed));

    let stopped_session = restore_compacting(&store, &provider);
    let resuming = stopped_session.resume(None).unwrap();
    until(|| provider.requests().len() == 6).await;
    let stopped = stopped_session.abort().await;

    assert_eq!(stopped.target, Some(AbortTarget::ActiveCompaction));
    assert_eq!(resuming.await, Ok(ActionEnd::Aborted));
    let blocks = stopped_session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        [
            "user",
            "compaction_boundary",
            "text",
            "response",
            "compaction_marker",
            "user",
            "text",
            "abort",
            "resume",
            "abort"
        ]
    );
    assert!(matches!(&blocks[7], Block::Abort(abort) if abort.is_resumed));
    assert_eq!(provider.requests().len(), 6);
}

#[tokio::test(flavor = "local")]
async fn input_too_large_for_the_model_on_its_own_is_never_summarized_and_its_refused_request_fails_the_turn()
 {
    let provider = ScriptedRuntime::new([
        answer("first"),
        answer("summary of one"),
        too_long(),
        answer("summary of the first answer"),
        too_long(),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session(&provider, Vec::new(), &store).await;
    session
        .send(text("one"), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    // Twice the model's window.
    let huge = "h".repeat(8_000);

    let failed = session.send(text(&huge), turn("t2")).unwrap().await;

    // The pass before the turn summarized what the first request carried;
    // the pass after the refusal found no answered request since, and
    // summarized what came before the message. Neither summary request
    // carried the message; each request of the turn carried it after a
    // summary, and the second refusal failed the turn.
    assert_eq!(
        failed.unwrap_err().code.as_deref(),
        Some("context_length_exceeded")
    );
    let requests = provider.requests();
    assert_eq!(requests.len(), 5, "{requests:?}");
    let message = user_item(&huge);
    assert!(
        requests
            .iter()
            .filter(|request| is_copy(request))
            .all(|request| !request.items.contains(&message))
    );
    assert_eq!(
        requests[2].items.as_ref(),
        [
            boundary_items(&["one"], "summary of one"),
            vec![answer_item("small-model", "first"), message.clone()],
        ]
        .concat()
    );
    assert_eq!(
        requests[4].items.as_ref(),
        [
            boundary_items(&["one"], "summary of the first answer"),
            vec![message],
        ]
        .concat()
    );
    // The message stays where it was, before the failure.
    assert_eq!(
        kinds(&session.transcript().blocks),
        [
            "user",
            "compaction_boundary",
            "text",
            "response",
            "compaction_boundary",
            "user",
            "compaction_marker",
            "compaction_marker",
            "error"
        ]
    );
}

#[tokio::test(flavor = "local")]
async fn a_resume_with_a_pending_switch_to_a_smaller_window_unwinds_then_compacts_once_with_the_model_before_it()
 {
    let old_answer = format!("old answer {}", "x".repeat(3_200));
    let provider = ScriptedRuntime::new([
        answer(&old_answer),
        answer("second answer"),
        partial_then_hang("partial answer"),
        answer("switch summary"),
        answer("continued after the switch"),
    ]);
    let store = MemoryTreeStore::new();
    let config = SessionConfig {
        compaction: only_when_asked(),
        ..SessionConfig::default()
    };
    let written = start(&provider, Vec::new(), &store, config).await;
    written
        .send(text("old question"), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    written
        .send(text("second question"), turn("t2"))
        .unwrap()
        .await
        .unwrap();
    let running = written.send(text("follow-up"), turn("t3")).unwrap();
    until(|| written.transcript().blocks.len() == 8).await;
    written.abort().await;
    running.await.unwrap();
    let session = restore_compacting(&store, &provider);
    session
        .update_model(ModelSwitch {
            model: Box::new(small_model()),
            runtime: None,
        })
        .unwrap();

    session.resume(None).unwrap().await.unwrap();

    // One pass, with the model before the switch; the new model continues.
    let requests = provider.requests();
    let [summary, continuation] = &requests[3..] else {
        panic!("{requests:?}");
    };
    assert!(is_copy(summary) && !is_copy(continuation));
    assert_eq!(summary.model_id, "test-model");
    assert_eq!(continuation.model_id, "small-model");
    assert_eq!(
        continuation.items.as_ref(),
        [
            boundary_items(&["old question", "second question"], "switch summary"),
            vec![
                answer_item("test-model", "second answer"),
                user_item("follow-up"),
                answer_item("test-model", "partial answer"),
                user_item(RESUME_TEXT),
            ],
        ]
        .concat()
    );
    // The unwind kept the stop, the switch's pass summarized what the
    // second request carried, and the one `resume` block follows the
    // marker.
    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        [
            "user",
            "text",
            "response",
            "user",
            "compaction_boundary",
            "text",
            "response",
            "user",
            "text",
            "abort",
            "compaction_marker",
            "resume",
            "text",
            "response"
        ]
    );
    let (Block::CompactionBoundary(boundary), Block::CompactionMarker(marker)) =
        (&blocks[4], &blocks[10])
    else {
        unreachable!();
    };
    assert_eq!(marker.boundary_id, boundary.id);
    assert!(matches!(&blocks[9], Block::Abort(abort) if abort.is_resumed));
}

#[tokio::test(flavor = "local")]
async fn a_round_whose_usage_with_its_cache_reaches_the_threshold_summarizes_what_its_request_carried_and_its_tool_never_runs_again()
 {
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![
            event::tool_call("call-1", "count", json!({})),
            // Only the cache write puts the usage over the threshold of 500.
            ProviderEvent::Response(Some(TokenUsage {
                input_tokens: 1,
                output_tokens: 1,
                cache_read_tokens: 0,
                cache_write_tokens: 850,
            })),
        ]),
        answer("tool summary"),
        answer("continued"),
    ]);
    let store = MemoryTreeStore::new();
    let (count, runs) = counted("count", "counted");
    let session = small_session(&provider, vec![count], &store).await;

    session
        .send(text("use the tool"), turn("t1"))
        .unwrap()
        .await
        .unwrap();

    assert_eq!(runs.get(), 1);
    let requests = provider.requests();
    let [_, summary, continuation] = requests.as_slice() else {
        panic!("{requests:?}");
    };
    // The summary request carries the call and its result, the instruction
    // after them as a message of its own, which a provider puts in the
    // results' message; the pass summarizes what the round's request
    // carried, the call and its result are kept, and the model continues
    // after them.
    let [message, call, result, instruction] = summary.items.as_ref() else {
        panic!("{:?}", summary.items);
    };
    assert_eq!(
        [message, instruction],
        [
            &user_item("use the tool"),
            &user_item(COMPACTION_SUMMARY_INSTRUCTION)
        ]
    );
    assert_eq!(item_kinds(&[call.clone(), result.clone()]), ["tool_use", "tool_result"]);
    assert_eq!(
        item_kinds(&continuation.items),
        [
            "user_message",
            "user_message",
            "user_message",
            "tool_use",
            "tool_result",
            "user_message"
        ]
    );
    assert_eq!(
        continuation.items[..3],
        boundary_items(&["use the tool"], "tool summary")
    );
    assert!(matches!(
        &continuation.items[4],
        InferenceItem::ToolResult { output, .. } if *output == sent_texts(&["counted"])
    ));
    assert_eq!(continuation.items[5], user_item(RESUME_TEXT));
    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        [
            "user",
            "compaction_boundary",
            "tool_call:completed",
            "response",
            "compaction_marker",
            "resume",
            "text",
            "response"
        ]
    );
    // The usage is recorded with its cache, and no request carries it.
    assert!(
        matches!(&blocks[3], Block::Response(response) if response.usage.is_some_and(|usage| usage.cache_write_tokens == 850))
    );
}

// Two turns of one scripted request each: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_response_without_usage_compacts_nothing_and_leaves_the_estimate_on_the_usage_before_it() {
    // The second response reports no usage, as Claude Code's does when its
    // stream held no call's own count: it is recorded without one, starts no
    // pass, and the estimate stays anchored on the first response's 400
    // tokens, never on none.
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![event::text("one"), event::response(400, 0)]),
        Turn::Events(vec![event::text("two"), ProviderEvent::Response(None)]),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session(&provider, vec![], &store).await;
    session.send(text("first"), turn("t1")).unwrap().await.unwrap();

    session.send(text("second"), turn("t2")).unwrap().await.unwrap();

    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        ["user", "text", "response", "user", "text", "response"]
    );
    assert!(matches!(&blocks[5], Block::Response(response) if response.usage.is_none()));
    let usage = session.context_usage().await.unwrap();
    assert!(usage.tokens > 400, "{usage:?}");
    assert_eq!(provider.remaining(), 0);
}

// One turn of two scripted requests: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_response_that_reports_more_input_than_the_models_window_compacts_nothing() {
    // 1,100 tokens of input cannot be one request's in a window of 1,000,
    // as when a provider sums the calls of its run: the round goes on
    // without a pass, where its usage would otherwise be over the threshold
    // of 500.
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![
            event::tool_call("call-1", "count", json!({})),
            ProviderEvent::Response(Some(TokenUsage {
                input_tokens: 100,
                output_tokens: 1,
                cache_read_tokens: 600,
                cache_write_tokens: 400,
            })),
        ]),
        answer("continued"),
    ]);
    let store = MemoryTreeStore::new();
    let (count, runs) = counted("count", "counted");
    let session = small_session(&provider, vec![count], &store).await;

    session
        .send(text("use the tool"), turn("t1"))
        .unwrap()
        .await
        .unwrap();

    assert_eq!(runs.get(), 1);
    assert_eq!(provider.remaining(), 0);
    assert_eq!(
        kinds(&session.transcript().blocks),
        ["user", "tool_call:completed", "response", "text", "response"]
    );
}

#[tokio::test(flavor = "local")]
async fn a_pass_with_nothing_to_summarize_but_the_last_summary_sends_no_request() {
    let provider = ScriptedRuntime::new([
        answer("first answer"),
        answer("summary one"),
        Turn::Events(vec![event::error(
            "the key expired",
            Some(ErrorCode::AuthExpired),
        )]),
        answer("summary two"),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session(&provider, Vec::new(), &store).await;

    session.compact().unwrap().await.unwrap();

    assert!(provider.requests().is_empty());
    assert!(session.transcript().blocks.is_empty());
    assert_eq!(session.phase(), SessionPhase::Idle);

    // A turn fails after a pass. With no request answered since, the next
    // pass summarizes the summary and the answer kept with it, and keeps
    // the failed message: the pass after it would hold the last summary
    // alone.
    session
        .send(text("one"), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    session.compact().unwrap().await.unwrap();
    let failed = session.send(text("two"), turn("t2")).unwrap().await;
    assert!(failed.is_err());
    session.compact().unwrap().await.unwrap();
    session.compact().unwrap().await.unwrap();

    assert_eq!(provider.requests().len(), 4);
    assert_eq!(
        kinds(&session.transcript().blocks),
        [
            "user",
            "compaction_boundary",
            "text",
            "response",
            "compaction_marker",
            "compaction_boundary",
            "user",
            "error",
            "compaction_marker"
        ]
    );
}

#[tokio::test(flavor = "local")]
async fn a_context_source_sees_only_its_blocks_after_the_last_boundary_and_tells_the_model_again() {
    let provider = ScriptedRuntime::new([
        answer("first answer"),
        answer("second answer"),
        answer("summary"),
        answer("third answer"),
    ]);
    let store = MemoryTreeStore::new();
    let runtime = TestRuntime {
        execution: Some("Runs on the Cloud."),
        ..test_runtime(Vec::new())
    };
    let seen = runtime.seen.clone();
    let session = start_on(&provider, runtime, &store, SessionConfig::default()).await;

    for (message, id) in [("one", "t1"), ("two", "t2")] {
        session
            .send(text(message), turn(id))
            .unwrap()
            .await
            .unwrap();
    }
    session.compact().unwrap().await.unwrap();
    session
        .send(text("three"), turn("t3"))
        .unwrap()
        .await
        .unwrap();

    // Told once before the first request; the summary replaced that block,
    // so after the pass the source is shown nothing and tells it again.
    let told = vec!["execution: Runs on the Cloud.".to_owned()];
    assert_eq!(*seen.borrow(), [vec![], told.clone(), vec![]]);
    let blocks = session.transcript().blocks;
    let contexts: Vec<_> = blocks
        .iter()
        .filter_map(|block| match block {
            Block::Context(context) => Some((context.source.as_str(), context.text.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        contexts,
        [
            ("execution", "Runs on the Cloud."),
            ("execution", "Runs on the Cloud.")
        ]
    );
    let last = provider.requests().pop().unwrap();
    assert!(
        format!("{:?}", last.items).contains("Runs on the Cloud."),
        "{:?}",
        last.items
    );
}

#[tokio::test(flavor = "local")]
async fn a_second_pass_folds_the_first_summary_in_and_a_restored_session_replays_only_the_last() {
    // Answers of about 300 tokens fill the 1,000-token window, whose
    // threshold is 500.
    let long = |name: &str| format!("{name} answer {}", "a".repeat(1_200));
    let provider = ScriptedRuntime::new([
        answer(&long("first")),
        answer(&long("second")),
        answer("summary one"),
        answer(&long("third")),
        answer("summary two"),
        answer("fourth answer"),
        answer("answer after the restore"),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session(&provider, Vec::new(), &store).await;
    for (message, id) in [("first", "t1"), ("second", "t2"), ("third", "t3")] {
        session.send(text(message), turn(id)).unwrap().await.unwrap();
    }

    session.send(text("fourth"), turn("t4")).unwrap().await.unwrap();

    // The second window starts at the first boundary: the first summary
    // folds into the second, nothing before it is summarized again, and the
    // messages the first boundary kept stay kept, before the next ones.
    let requests = provider.requests();
    let [.., third_request, second_summary, fourth_request] = requests.as_slice() else {
        panic!("{requests:?}");
    };
    assert!(is_copy(second_summary));
    assert_eq!(
        second_summary.items.as_ref(),
        [
            third_request.items.to_vec(),
            vec![
                answer_item("small-model", &long("third")),
                instructed_item("fourth"),
            ]
        ]
        .concat()
    );
    assert_eq!(
        second_summary.items.as_ref(),
        [
            boundary_items(&["first", "second"], "summary one"),
            vec![
                answer_item("small-model", &long("second")),
                user_item("third"),
                answer_item("small-model", &long("third")),
                instructed_item("fourth"),
            ],
        ]
        .concat()
    );
    assert_eq!(
        fourth_request.items.as_ref(),
        [
            boundary_items(&["first", "second", "third"], "summary two"),
            vec![
                answer_item("small-model", &long("third")),
                user_item("fourth"),
            ],
        ]
        .concat()
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
        .send(text("after the restore"), turn("t5"))
        .unwrap()
        .await
        .unwrap();

    let requests = provider.requests();
    assert_eq!(
        requests.last().unwrap().items.as_ref(),
        [
            fourth_request.items.to_vec(),
            vec![
                answer_item("small-model", "fourth answer"),
                user_item("after the restore"),
            ],
        ]
        .concat()
    );
}

#[tokio::test(flavor = "local")]
async fn input_that_arrives_during_a_pass_waits_outside_the_summary_for_the_first_request_after_it()
{
    let (first_summary, first_release) =
        gated_turn(vec![event::text("summary one"), event::response(0, 0)]);
    let (second_summary, second_release) =
        gated_turn(vec![event::text("summary two"), event::response(0, 0)]);
    let first_answer = format!("first answer {}", "f".repeat(1_600));
    let provider = ScriptedRuntime::new([
        answer(&first_answer),
        answer("second answer"),
        first_summary,
        answer("third answer"),
        answer("fourth answer"),
        second_summary,
        answer("fifth answer"),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session(&provider, Vec::new(), &store).await;
    // The third message leaves the 150 estimated tokens a 1,000-token
    // window keeps after the end of the second request, which the pass
    // ends at.
    let third = "z".repeat(600);
    let fourth = "w".repeat(200);
    for (message, id) in [("first", "t1"), ("second", "t2")] {
        session.send(text(message), turn(id)).unwrap().await.unwrap();
    }
    let third_turn = session.send(text(&third), turn("t3")).unwrap();
    until(|| provider.requests().len() == 3).await;

    // During the pass before t3's turn, a message waits in the queue and a
    // steer for t3.
    let fourth_turn = session.send(text(&fourth), turn("t4")).unwrap();
    session
        .steer(text("mind the tests"), BlockId::try_from("s1").unwrap())
        .unwrap();
    assert_eq!(session.phase(), SessionPhase::Compacting);
    assert_eq!(
        session.queued_messages(),
        [QueuedMessage {
            id: turn("t4"),
            content: text(&fourth),
        }]
    );
    let steers = session.pending_steers();
    assert_eq!(steers.len(), 1);
    assert_eq!(steers[0].turn_id, turn("t3"));
    let _ = first_release.send(());
    third_turn.await.unwrap();
    fourth_turn.await.unwrap();

    // The summary holds neither; t3's first request carries the steer, and
    // t4 runs after t3.
    let requests = provider.requests();
    assert_eq!(
        item_kinds(&requests[2].items),
        ["user_message", "assistant_text", "user_message", "assistant_text", "user_message"]
    );
    assert_eq!(requests[2].items.last(), Some(&instructed_item(&third)));
    let steer = InferenceItem::UserSteer {
        content: sent_text("mind the tests"),
    };
    let after_first = [
        boundary_items(&["first", "second"], "summary one"),
        vec![
            answer_item("small-model", "second answer"),
            user_item(&third),
            steer,
        ],
    ]
    .concat();
    assert_eq!(requests[3].items.as_ref(), after_first.as_slice());
    assert_eq!(
        requests[4].items.as_ref(),
        [
            after_first.clone(),
            vec![
                answer_item("small-model", "third answer"),
                user_item(&fourth),
            ],
        ]
        .concat()
    );

    // During a `compact` action, a message waits in the queue, and its turn
    // starts from the new summary, which keeps the steer as a message of
    // its own.
    let compacting = session.compact().unwrap();
    until(|| provider.requests().len() == 6).await;
    let fifth_turn = session.send(text("after the pass"), turn("t5")).unwrap();
    assert_eq!(session.phase(), SessionPhase::Compacting);
    assert_eq!(session.queued_messages().len(), 1);
    let _ = second_release.send(());
    compacting.await.unwrap();
    fifth_turn.await.unwrap();

    assert_eq!(
        provider.requests()[6].items.as_ref(),
        [
            // Newest first within the 100 tokens a 1,000-token window keeps:
            // the fourth message and the steer whole, the third cut to what
            // is left, and the first two left to the summary.
            boundary_items(
                &[&third[..184], "mind the tests", &fourth],
                "summary two"
            ),
            vec![
                answer_item("small-model", "fourth answer"),
                user_item("after the pass"),
            ],
        ]
        .concat()
    );
}

/// A question of 101 estimated tokens that starts with `name`.
fn question(name: &str) -> String {
    format!("{name} {}", "q".repeat(400))
}

#[tokio::test(flavor = "local")]
async fn an_edit_before_or_after_compaction_boundaries_replays_only_the_summaries_of_what_it_keeps()
{
    let provider = ScriptedRuntime::new([
        answer("answer A"),
        answer("answer B"),
        answer("summary one"),
        answer("answer C"),
        answer("summary two"),
        // One answer for each replacement, in the order below.
        answer("replaced A"),
        answer("replaced B"),
        answer("replaced C"),
    ]);
    let store = MemoryTreeStore::new();
    let written = small_session_with(&provider, Vec::new(), &store, only_when_asked()).await;
    for name in ["A", "B"] {
        written
            .send(text(&question(name)), turn(name))
            .unwrap()
            .await
            .unwrap();
    }
    written.compact().unwrap().await.unwrap();
    written
        .send(text(&question("C")), turn("C"))
        .unwrap()
        .await
        .unwrap();
    written.compact().unwrap().await.unwrap();
    // Each boundary stands where the content of the request before it
    // ended, so a message comes before the boundary of the pass that
    // followed its answer.
    let before = written.transcript().blocks;
    assert_eq!(
        kinds(&before),
        [
            "user",
            "text",
            "response",
            "user",
            "compaction_boundary",
            "text",
            "response",
            "compaction_marker",
            "user",
            "compaction_boundary",
            "text",
            "response",
            "compaction_marker"
        ]
    );

    for (name, index, kept) in [
        ("A", 0, Vec::new()),
        (
            "B",
            3,
            vec![
                user_item(&question("A")),
                answer_item("small-model", "answer A"),
            ],
        ),
        (
            "C",
            8,
            [
                // B's question, cut to the 100 tokens of the user's words
                // a 1,000-token window keeps; A's is left to the summary.
                boundary_items(&[&question("B")[..400]], "summary one"),
                vec![answer_item("small-model", "answer B")],
            ]
            .concat(),
        ),
    ] {
        let session = restore_compacting(&store, &provider);
        let replacement = format!("{name}2");

        session
            .edit_and_send(edit_of(&session, name, "op1", &replacement))
            .await
            .unwrap();
        session.settled().await;

        // The summaries after the message went with it; one before it stands
        // for what it summarized.
        let blocks = session.transcript().blocks;
        assert_eq!(blocks[..index], before[..index], "editing {name}");
        assert_eq!(kinds(&blocks[index..]), ["user", "text", "response"]);
        let expected = [kept, vec![user_item(&replacement)]].concat();
        assert_eq!(
            provider.requests().last().unwrap().items.as_ref(),
            expected.as_slice(),
            "editing {name}"
        );
    }
}

#[tokio::test(flavor = "local")]
async fn an_edit_that_cuts_a_marker_anchors_no_estimate_on_a_usage_measured_before_its_summary() {
    let provider = ScriptedRuntime::new([
        answer("answer A"),
        // Measured over the whole history, before the summary replaced A.
        Turn::Events(vec![event::text("answer B"), event::response(900, 0)]),
        Turn::Events(vec![event::error(
            "the key expired",
            Some(ErrorCode::AuthExpired),
        )]),
        answer("summary one"),
        answer("replaced C"),
    ]);
    let store = MemoryTreeStore::new();
    let written = small_session_with(&provider, Vec::new(), &store, only_when_asked()).await;
    for name in ["A", "B"] {
        written
            .send(text(&question(name)), turn(name))
            .unwrap()
            .await
            .unwrap();
    }
    let failed = written.send(text(&question("C")), turn("C")).unwrap().await;
    assert!(failed.is_err());
    // C was never answered: the pass summarizes what A's request carried,
    // the latest whose end leaves 150 estimated tokens after it, and keeps
    // A's answer, B and C after the boundary.
    written.compact().unwrap().await.unwrap();
    assert_eq!(
        kinds(&written.transcript().blocks),
        [
            "user",
            "compaction_boundary",
            "text",
            "response",
            "user",
            "text",
            "response",
            "user",
            "error",
            "compaction_marker"
        ]
    );
    let session = restore_compacting(&store, &provider);

    session
        .edit_and_send(edit_of(&session, "C", "op1", "C2"))
        .await
        .unwrap();
    session.settled().await;

    // The marker went with C, and B's usage, over the threshold of 500,
    // stays invalid: no pass runs before the replacement's turn. The one
    // summary request held the whole history, five items.
    assert_eq!(summary_sizes(&provider), [5]);
    assert_eq!(
        provider.requests().last().unwrap().items.as_ref(),
        [
            boundary_items(&[&question("A")[..400]], "summary one"),
            vec![
                answer_item("small-model", "answer A"),
                user_item(&question("B")),
                answer_item("small-model", "answer B"),
                user_item("C2"),
            ],
        ]
        .concat()
    );
}

#[tokio::test(flavor = "local")]
async fn a_pass_after_an_edit_summarizes_only_the_history_the_edit_kept() {
    let first = format!("A {}", "q".repeat(3_200));
    let provider = ScriptedRuntime::new([
        answer("answer A"),
        answer("answer B"),
        answer("answer C"),
        answer("summary of A"),
        answer("replaced B"),
    ]);
    let store = MemoryTreeStore::new();
    let written = small_session_with(&provider, Vec::new(), &store, only_when_asked()).await;
    written
        .send(text(&first), turn("A"))
        .unwrap()
        .await
        .unwrap();
    for name in ["B", "C"] {
        written.send(text(name), turn(name)).unwrap().await.unwrap();
    }
    let session = restore_compacting(&store, &provider);

    session
        .edit_and_send(edit_of(&session, "B", "op1", "B2"))
        .await
        .unwrap();
    session.settled().await;

    // What the edit kept is over the threshold: the pass asks with the
    // replacement's request and summarizes what the request of A carried,
    // and no request holds an answer the edit removed.
    let requests = provider.requests();
    let [summary, replacement] = &requests[3..] else {
        panic!("{requests:?}");
    };
    assert_eq!(
        summary.items.as_ref(),
        [
            user_item(&first),
            answer_item("small-model", "answer A"),
            instructed_item("B2"),
        ]
    );
    assert_eq!(
        replacement.items.as_ref(),
        [
            boundary_items(&[&first[..400]], "summary of A"),
            vec![answer_item("small-model", "answer A"), user_item("B2")],
        ]
        .concat()
    );
}

/// A round that calls `shoot`.
fn shot_round(call: &str) -> Turn {
    Turn::Events(vec![
        event::tool_call(call, "shoot", json!({})),
        event::response(1, 1),
    ])
}

/// A tool whose result is the screenshot `png`.
fn shoot(png: B64Bytes) -> (String, Invoke) {
    tool("shoot", move |_| {
        let image = ResultPart::Image(MediaBytes {
            data: png.clone(),
            media_type: "image/png".into(),
        });
        Box::pin(async move {
            Ok(ToolOutcome {
                output: vec![image],
                ..output("")
            })
        })
    })
}

/// A vendor that takes `images` images or `body_bytes` bytes a request.
fn taking(images: Option<u32>, body_bytes: Option<u64>) -> RequestLimits {
    RequestLimits {
        body_bytes,
        images,
        tool_results: demi_provider_common::ToolResultKinds::ALL,
    }
}

/// Whether `blocks` hold a compaction.
fn compacted(blocks: &[Block]) -> bool {
    blocks
        .iter()
        .any(|block| matches!(block, Block::CompactionBoundary(_) | Block::CompactionMarker(_)))
}

/// A session on `limits` whose model reads PNG, after four rounds that
/// each returned `shot` and an answer. The summary a pass would ask for is
/// scripted after them, so that one shows as a request rather than a wait.
async fn after_four_shots(
    shot: &B64Bytes,
    limits: Option<RequestLimits>,
) -> (AgentSession, ScriptedRuntime) {
    let rounds = [
        shot_round("shot-1"),
        shot_round("shot-2"),
        shot_round("shot-3"),
        shot_round("shot-4"),
        answer("four shots"),
        answer("summary of the shots"),
    ];
    let provider = match limits {
        Some(limits) => ScriptedRuntime::new(rounds).with_limits(limits),
        None => ScriptedRuntime::new(rounds),
    };
    let store = MemoryTreeStore::new();
    let tools = vec![shoot(shot.clone())];
    let session = start(&provider, tools, &store, SessionConfig::default()).await;
    session
        .update_model(ModelSwitch {
            model: Box::new(model_reading("stub", "model-a", &[FileExtension::Png])),
            runtime: None,
        })
        .unwrap();
    session
        .send(text("watch the screen"), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    (session, provider)
}

// Four sessions of a few scripted requests each: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn screenshots_near_a_vendors_image_or_byte_limit_never_compact_a_history_under_the_token_threshold() {
    let header = b"\x89PNG\r\n\x1a\n\0\0\0\x01".as_slice();
    // Four screenshots of 4,000 bytes of base64 each, a tenth of the window
    // in tokens. The vendor takes five images, or 20,000 bytes, a request,
    // so a request that carries four is at 80% of its limit.
    let shot = B64Bytes::from([header, &[0; 2_988]].concat());
    let cases = [
        ("images", taking(Some(5), None)),
        ("bytes", taking(None, Some(20_000))),
    ];
    for (limit, limits) in cases {
        // Compaction runs on tokens only: no summary request, no pass, and
        // the last request carries the four screenshots.
        let (session, provider) = after_four_shots(&shot, Some(limits)).await;
        let requests = provider.requests();
        assert_eq!(requests.len(), 5, "{limit}");
        let results = requests[4]
            .items
            .iter()
            .filter(|item| matches!(item, InferenceItem::ToolResult { .. }))
            .count();
        assert_eq!(results, 4, "{limit}");
        assert!(!compacted(&session.transcript().blocks), "{limit}");

        // A switch to that vendor from one without limits does not compact
        // either, with the model before it or after.
        let (session, provider) = after_four_shots(&shot, None).await;
        let after = ScriptedRuntime::new([answer("after the switch")]).with_limits(limits);
        session
            .update_model(ModelSwitch {
                model: Box::new(model_reading("other", "model-b", &[FileExtension::Png])),
                runtime: Some(Box::new(after.clone())),
            })
            .unwrap();
        session
            .send(text("and now?"), turn("t2"))
            .unwrap()
            .await
            .unwrap();

        assert_eq!(provider.requests().len(), 5, "{limit}");
        assert_eq!(after.requests().len(), 1, "{limit}");
        assert!(!compacted(&session.transcript().blocks), "{limit}");
    }
}

#[tokio::test(flavor = "local")]
async fn a_request_refused_for_its_size_is_not_compacted() {
    // An HTTP 413: the vendor's body or image limit, not the model's
    // window.
    let refused = Turn::Events(vec![ProviderEvent::Error(ProviderFailure {
        message: "Request exceeds the maximum size".into(),
        code: ErrorCode::from_http(413, ""),
        diagnostics: None,
        retry_after: None,
    })]);
    // A summary and an answer after it stay unasked: no pass runs.
    let provider = ScriptedRuntime::new([
        answer("first"),
        refused,
        answer("summary of the first message"),
        answer("second"),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session(&provider, Vec::new(), &store).await;
    session
        .send(text("one"), turn("t1"))
        .unwrap()
        .await
        .unwrap();

    let failed = session.send(text("two"), turn("t2")).unwrap().await;

    assert_eq!(
        failed.unwrap_err().code.as_deref(),
        Some("request_too_large")
    );
    assert_eq!(provider.requests().len(), 2);
    assert_eq!(provider.remaining(), 2);
    assert!(!compacted(&session.transcript().blocks));
}

// One session of five scripted requests: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_summary_request_refused_for_its_size_fails_the_pass_without_a_smaller_window() {
    let refused = Turn::Events(vec![ProviderEvent::Error(ProviderFailure {
        message: "Request exceeds the maximum size".into(),
        code: ErrorCode::from_http(413, ""),
        diagnostics: None,
        retry_after: None,
    })]);
    let provider = ScriptedRuntime::new([
        answer("one"),
        answer("two"),
        answer("three"),
        refused,
        // A summary of a smaller window stays unasked.
        answer("summary of the first half"),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session_with(&provider, Vec::new(), &store, only_when_asked()).await;
    for name in ["A", "B", "C"] {
        session.send(text(name), turn(name)).unwrap().await.unwrap();
    }

    let failed = session.compact().unwrap().await;

    assert_eq!(
        failed.unwrap_err().code.as_deref(),
        Some("request_too_large")
    );
    assert_eq!(summary_sizes(&provider).len(), 1);
    assert_eq!(provider.remaining(), 1);
    assert!(!compacted(&session.transcript().blocks));
}

// One session and four scripted requests: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_summary_request_gives_the_claude_code_cli_its_instruction_inside_the_message_the_cli_recorded()
 {
    // The CLI recorded the second message, after the metadata it writes
    // before one, before the vendor refused its request; the pass after the
    // refusal asks with that message last.
    let date = json!({
        "type": "attachment",
        "uuid": "recorded-date",
        "attachment": { "type": "date_change", "newDate": "2026-10-14" },
    });
    let meta = json!({
        "type": "user",
        "uuid": "recorded-meta",
        "isMeta": true,
        "message": { "role": "user", "content": "<model-switch>small-model</model-switch>" },
    });
    let recorded = json!({
        "type": "user",
        "uuid": "recorded-two",
        "message": { "role": "user", "content": [{ "type": "text", "text": "two" }] },
    });
    let provider = ScriptedRuntime::new([
        answer("first"),
        Turn::Events(vec![
            ProviderEvent::Entries {
                of: EntriesOf::Item(2),
                entries: vec![date.clone(), meta.clone(), recorded],
            },
            ProviderEvent::Error(ProviderFailure {
                message: "prompt is too long".into(),
                code: Some(ErrorCode::ContextLengthExceeded),
                diagnostics: None,
                retry_after: None,
            }),
        ]),
        answer("summary of the first message"),
        answer("second"),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session(&provider, Vec::new(), &store).await;
    session.send(text("one"), turn("t1")).unwrap().await.unwrap();

    session.send(text("two"), turn("t2")).unwrap().await.unwrap();

    // The session file a new CLI process resumes ends with the metadata as
    // the CLI wrote it, then one message, the recorded second with the
    // instruction as one more text block: two messages in a row would make
    // the CLI add a line break to the first one's text.
    let requests = provider.requests();
    let summary = &requests[2];
    assert!(is_copy(summary));
    let file = demi_provider_claude_code::testing::session(summary, "2026-10-14T12:00:00.000Z");
    let entries: Vec<serde_json::Value> = String::from_utf8(file)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let [.., written_date, written_meta, last] = entries.as_slice() else {
        panic!("{entries:#?}");
    };
    for (written, recorded) in [(written_date, &date), (written_meta, &meta)] {
        assert_eq!(written["uuid"], recorded["uuid"]);
        assert_eq!(written["attachment"], recorded["attachment"]);
        assert_eq!(written["message"], recorded["message"]);
    }
    assert_eq!(last["uuid"], "recorded-two");
    assert_eq!(
        last["message"]["content"],
        json!([
            { "type": "text", "text": "two" },
            { "type": "text", "text": COMPACTION_SUMMARY_INSTRUCTION },
        ])
    );
}

#[tokio::test(flavor = "local")]
async fn a_request_refused_for_its_tokens_compacts_once_and_goes_again_and_a_second_refusal_fails_the_turn()
 {
    let provider = ScriptedRuntime::new([
        answer("first"),
        too_long(),
        answer("summary of the first message"),
        answer("second"),
        too_long(),
        answer("summary of the second message"),
        too_long(),
    ]);
    let store = MemoryTreeStore::new();
    let session = small_session(&provider, Vec::new(), &store).await;
    let retries = Rc::new(Cell::new(0));
    let _listener = session.subscribe({
        let retries = retries.clone();
        move |event| {
            if let SessionEvent::RetryScheduled { .. } = event {
                retries.set(retries.get() + 1);
            }
        }
    });
    session
        .send(text("one"), turn("t1"))
        .unwrap()
        .await
        .unwrap();

    session
        .send(text("two"), turn("t2"))
        .unwrap()
        .await
        .unwrap();

    // The refused request left nothing behind; the pass asked with it and
    // the instruction joined to its message, summarized what the first
    // request carried, and the request went again from the summary.
    let requests = provider.requests();
    let [first, refused, summary, again] = &requests[..4] else {
        panic!("{requests:?}");
    };
    assert_eq!(
        summary.items.as_ref(),
        [
            first.items.to_vec(),
            vec![
                answer_item("small-model", "first"),
                instructed_item("two")
            ]
        ]
        .concat()
    );
    assert_eq!(
        refused.items.as_ref(),
        [
            user_item("one"),
            answer_item("small-model", "first"),
            user_item("two")
        ]
    );
    assert_eq!(
        again.items.as_ref(),
        [
            boundary_items(&["one"], "summary of the first message"),
            vec![answer_item("small-model", "first"), user_item("two")],
        ]
        .concat()
    );
    assert_eq!(retries.get(), 0);
    assert!(!kinds(&session.transcript().blocks).contains(&"error".to_owned()));

    // Refused again after its pass, the request fails the turn.
    let failed = session.send(text("three"), turn("t3")).unwrap().await;

    assert_eq!(
        failed.unwrap_err().code.as_deref(),
        Some("context_length_exceeded")
    );
    assert_eq!(provider.remaining(), 0);
    assert_eq!(
        kinds(&session.transcript().blocks)
            .last()
            .map(String::as_str),
        Some("error")
    );
}

/// A round that calls `work` and reports `input_tokens` of usage.
fn work_round(call: &str, input_tokens: u64) -> Turn {
    Turn::Events(vec![
        event::tool_call(call, "work", json!({})),
        event::response(input_tokens, 0),
    ])
}

// Four sessions of four scripted requests each: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn each_models_token_threshold_leaves_room_for_its_answer_and_the_summary_request() {
    for (window, output_limit, threshold) in [
        (1_000_000, 128_000, 967_000),
        (200_000, 64_000, 167_000),
        (128_000, 8_000, 107_000),
        // Half the window: room for the answer and the summary request
        // would leave less.
        (32_000, 32_000, 16_000),
    ] {
        // A usage one token under the threshold goes on; one at it compacts
        // after the round's tool.
        let provider = ScriptedRuntime::new([
            work_round("c1", threshold - 1),
            work_round("c2", threshold),
            answer("summary"),
            answer("done"),
        ]);
        let store = MemoryTreeStore::new();
        let work = tool("work", |_| Box::pin(async { Ok(output("worked")) }));
        let session = start(&provider, vec![work], &store, SessionConfig::default()).await;
        let mut model = model_of("stub", "sized-model");
        model.model.context_window = window;
        model.model.output_limit = Some(output_limit);
        session
            .update_model(ModelSwitch {
                model: Box::new(model),
                runtime: None,
            })
            .unwrap();

        session.send(text("go"), turn("t1")).unwrap().await.unwrap();

        let copies: Vec<bool> = provider.requests().iter().map(is_copy).collect();
        assert_eq!(copies, [false, false, true, false], "{window}: {threshold}");
    }
}

/// A turn of five calls in a 1,000-token window, each returning `output`;
/// the fifth round's usage reaches the threshold, so a pass follows its
/// call. Returns the session's requests.
async fn five_calls(output_text: String) -> Vec<demi_provider_common::InferenceRequest> {
    let provider = ScriptedRuntime::new([
        work_round("c1", 0),
        work_round("c2", 0),
        work_round("c3", 0),
        work_round("c4", 0),
        work_round("c5", 600),
        answer("summary of the first calls"),
        answer("done"),
    ]);
    let store = MemoryTreeStore::new();
    let work = tool("work", move |_| {
        let text = output_text.clone();
        Box::pin(async move { Ok(output(&text)) })
    });
    let session = small_session(&provider, vec![work], &store).await;
    session.send(text("go"), turn("t1")).unwrap().await.unwrap();
    assert_eq!(
        kinds(&session.transcript().blocks)
            .iter()
            .filter(|kind| *kind == "compaction_boundary")
            .count(),
        1
    );
    provider.requests()
}

// Two sessions of seven scripted requests: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn the_cut_keeps_the_recent_work_at_the_end_of_an_answered_request() {
    // Each call and its result weigh about 80 estimated tokens, so the
    // last two leave the 150 a 1,000-token window keeps: the window ends
    // where the request that carried the third call's result ended, and the
    // fourth and fifth calls stay as they were, each with its result.
    // The summary request is the fifth request, the fifth call and its
    // result, then the instruction.
    let requests = five_calls("o".repeat(240)).await;
    let (fifth, summary, continuation) = (&requests[4], &requests[5], &requests[6]);
    let (sent, after) = summary.items.split_at(fifth.items.len());
    assert_eq!(sent, fifth.items.as_ref());
    assert_eq!(item_kinds(after), ["tool_use", "tool_result", "user_message"]);
    assert_eq!(after.last(), Some(&user_item(COMPACTION_SUMMARY_INSTRUCTION)));
    assert_eq!(
        item_kinds(&continuation.items),
        [
            "user_message",
            "user_message",
            "user_message",
            "tool_use",
            "tool_result",
            "tool_use",
            "tool_result",
            "user_message"
        ]
    );
    assert_eq!(
        continuation.items[..3],
        boundary_items(&["go"], "summary of the first calls")
    );

    // When no end leaves that much, the window ends where the latest
    // answered request's content ended: only the fifth call stays.
    let requests = five_calls("ok".to_owned()).await;
    let (fifth, summary, continuation) = (&requests[4], &requests[5], &requests[6]);
    assert_eq!(summary.items[..fifth.items.len()], fifth.items[..]);
    assert_eq!(
        item_kinds(&continuation.items[3..]),
        ["tool_use", "tool_result", "user_message"]
    );
}

/// The content a kept message replays as.
fn kept_texts(boundary: &Block) -> Vec<Vec<String>> {
    let Block::CompactionBoundary(boundary) = boundary else {
        panic!("not a boundary: {boundary:?}");
    };
    boundary
        .kept
        .iter()
        .map(|kept| {
            kept.content
                .iter()
                .map(|part| match part {
                    UserContentBlock::Text { text } => text.clone(),
                    other => format!("{other:?}"),
                })
                .collect()
        })
        .collect()
}

// One session, two passes and messages of about 200 KB: a few
// milliseconds.
#[tokio::test(flavor = "local")]
async fn a_pass_keeps_the_users_newest_words_within_their_budget_beside_its_summary() {
    let (slow, releases, started) = gated_tool("slow");
    let provider = ScriptedRuntime::new([
        answer("noted"),
        answer("seen"),
        Turn::Events(vec![
            event::tool_call("call-1", "slow", json!({})),
            event::response(0, 0),
        ]),
        answer("done"),
        // A long report of 16,000 tokens in as many characters, which the
        // replay bound keeps whole, and the window keeps after its cut.
        answer(&"😀".repeat(16_000)),
        answer("summary one"),
        answer("going on"),
        answer("summary two"),
    ]);
    let store = MemoryTreeStore::new();
    let config = SessionConfig {
        compaction: only_when_asked(),
        ..SessionConfig::default()
    };
    let session = start(&provider, vec![slow], &store, config).await;
    session
        .update_model(ModelSwitch {
            model: Box::new(model_reading("stub", "model-a", &[FileExtension::Png])),
            runtime: None,
        })
        .unwrap();
    let photo = png(4, 3, 1);
    let mut held = HeldMedia::default();
    held.hold(BlobRef::of(&photo), photo.clone());
    session.hold_media(held);
    // 12,000 tokens and a screenshot; then 9,000 tokens. Their characters
    // are wide, so the replay bound keeps each whole and its estimate is
    // its own.
    let second = format!("Second s{}", "😀".repeat(11_998));
    let third = format!("Third {}", "あ".repeat(11_998));
    let up_to = |tokens: usize| second[..second.floor_char_boundary(tokens * 4)].to_owned();
    session.send(text("Keep the old rule."), turn("t1")).unwrap().await.unwrap();
    let mut with_image = text(&second);
    with_image.push(UserContentBlock::Image {
        source: MediaSource::Ref {
            r#ref: BlobRef::of(&photo),
            media_type: "image/png".into(),
            width: None,
            height: None,
        },
    });
    session.send(with_image, turn("t2")).unwrap().await.unwrap();
    let running = session.send(text(&third), turn("t3")).unwrap();
    started.await.unwrap();
    session
        .steer(text("Mind the tests."), BlockId::try_from("s1").unwrap())
        .unwrap();
    session
        .accept_agent_message(agent_message("m1"))
        .await
        .unwrap();
    let _ = releases.borrow_mut().remove(0).send(());
    running.await.unwrap();
    session.send(text("Now report."), turn("t4")).unwrap().await.unwrap();

    session.compact().unwrap().await.unwrap();

    // Newest first: the last message, the steer and the third message
    // whole; the second crosses the 10,000 tokens a 100,000-token window
    // keeps, 10% of it, and keeps its text up to them, without its
    // screenshot; the first is left to the summary, and so is the agent's
    // message.
    let blocks = session.transcript().blocks;
    let boundary = blocks
        .iter()
        .find(|block| matches!(block, Block::CompactionBoundary(_)))
        .expect("the pass inserted a boundary");
    let kept = kept_texts(boundary);
    let edge = 10_000 - (third.len() / 4 + 4 + 3);
    assert_eq!(
        kept,
        [
            vec![up_to(edge)],
            vec![third.clone()],
            vec!["Mind the tests.".to_owned()],
            vec!["Now report.".to_owned()],
        ]
    );
    session.send(text("Go on."), turn("t5")).unwrap().await.unwrap();
    let requests = provider.requests();
    let next = requests.last().unwrap();
    // The next request carries the note, the kept messages oldest first,
    // each as a user message with the replay's bound on its text, then the
    // summary.
    let starts = |item: &InferenceItem, start: &str| {
        matches!(item, InferenceItem::UserMessage { content }
            if matches!(content.as_slice(), [UserPart::Text(text)] if text.starts_with(start)))
    };
    assert_eq!(next.items[0], user_item(COMPACTION_NOTE));
    assert!(starts(&next.items[1], "Second "), "{:?}", next.items[1]);
    assert!(starts(&next.items[2], "Third "), "{:?}", next.items[2]);
    assert_eq!(
        next.items[3..6],
        [
            user_item("Mind the tests."),
            user_item("Now report."),
            summary_item("summary one")
        ]
    );

    // The next pass starts from what this boundary kept, and the new
    // message pushes the edge further.
    session.compact().unwrap().await.unwrap();
    let blocks = session.transcript().blocks;
    let boundary = blocks
        .iter()
        .rfind(|block| matches!(block, Block::CompactionBoundary(_)))
        .expect("the pass inserted a boundary");
    let edge = edge - 2;
    assert_eq!(
        kept_texts(boundary),
        [
            vec![up_to(edge)],
            vec![third.clone()],
            vec!["Mind the tests.".to_owned()],
            vec!["Now report.".to_owned()],
            vec!["Go on.".to_owned()],
        ]
    );
}
