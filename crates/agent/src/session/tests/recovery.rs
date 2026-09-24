//! Failures and recovery (`failures-and-recovery.md`): transient retries,
//! `retry` and `resume`.

use demi_core::{BlockId, SteerBlock};
use demi_provider::ProviderFailure;

use super::*;
use crate::transcript::RESUME_TEXT;

fn failure(code: Option<ErrorCode>, retry_after: Option<Duration>) -> ProviderEvent {
    ProviderEvent::Error(ProviderFailure {
        message: "the vendor failed".into(),
        code,
        diagnostics: None,
        retry_after,
    })
}

/// Every retry report the session emits.
fn retries(session: &AgentSession) -> (Subscription, Rc<RefCell<Vec<SessionEvent>>>) {
    let reports = Rc::new(RefCell::new(Vec::new()));
    let subscription = session.subscribe({
        let reports = reports.clone();
        move |event| {
            if let SessionEvent::RetryScheduled { .. } = event {
                reports.borrow_mut().push(event.clone());
            }
        }
    });
    (subscription, reports)
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn transient_failures_before_output_are_retried_with_reports_and_leave_no_trace() {
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![failure(Some(ErrorCode::Overloaded), None)]),
        Turn::Events(vec![failure(
            Some(ErrorCode::RateLimit),
            Some(Duration::from_millis(1_500)),
        )]),
        Turn::Events(vec![event::text("made it"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    let (_subscription, reports) = retries(&session);

    let end = session.send(text("go"), turn("t1")).unwrap().await;

    assert_eq!(end, Ok(ActionEnd::Completed));
    assert_eq!(
        kinds(&session.transcript().blocks),
        ["user", "text", "response"]
    );
    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    let reports = reports.borrow();
    let [
        SessionEvent::RetryScheduled {
            attempt: 1,
            delay_ms: first_delay,
            code: first_code,
            diagnostics: Some(first),
        },
        SessionEvent::RetryScheduled {
            attempt: 2,
            delay_ms: 1_500,
            code: second_code,
            diagnostics: Some(second),
        },
    ] = reports.as_slice()
    else {
        panic!("{reports:?}");
    };
    assert!(*first_delay < 1_000);
    assert_eq!(first_code.as_deref(), Some("overloaded"));
    assert_eq!(second_code.as_deref(), Some("rate_limit"));
    // Each report names its attempt's request, and the source defaults to
    // unknown.
    assert_eq!(
        first.client_request_id.as_deref(),
        Some(requests[0].request_id.as_str())
    );
    assert_eq!(
        second.client_request_id.as_deref(),
        Some(requests[1].request_id.as_str())
    );
    assert_eq!(first.source, FailureSource::Unknown);
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn thinking_before_a_transient_failure_is_unwound_while_text_makes_it_terminal() {
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![
            ProviderEvent::ThinkingStart,
            event::thinking("plan the answer"),
            failure(Some(ErrorCode::Overloaded), None),
        ]),
        Turn::Events(vec![event::text("answer"), event::response(1, 1)]),
        Turn::Events(vec![
            event::text("half an answer"),
            failure(Some(ErrorCode::Overloaded), None),
        ]),
    ]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;

    session
        .send(text("first"), turn("t1"))
        .unwrap()
        .await
        .unwrap();
    let failed = session.send(text("second"), turn("t2")).unwrap().await;

    assert_eq!(failed.unwrap_err().code.as_deref(), Some("overloaded"));
    assert_eq!(
        kinds(&session.transcript().blocks),
        ["user", "text", "response", "user", "text", "error"]
    );
    assert_eq!(provider.requests().len(), 3);
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn four_transient_failures_end_the_turn_with_the_last_and_a_long_vendor_wait_is_not_retried()
{
    let overloaded = || Turn::Events(vec![failure(Some(ErrorCode::Overloaded), None)]);
    let provider = ScriptedRuntime::new([
        overloaded(),
        overloaded(),
        overloaded(),
        overloaded(),
        Turn::Events(vec![failure(
            Some(ErrorCode::RateLimit),
            Some(Duration::from_secs(31)),
        )]),
    ]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    let (_subscription, reports) = retries(&session);

    let exhausted = session.send(text("go"), turn("t1")).unwrap().await;
    let delays: Vec<(u32, u64)> = reports
        .borrow()
        .iter()
        .map(|report| match report {
            SessionEvent::RetryScheduled {
                attempt, delay_ms, ..
            } => (*attempt, *delay_ms),
            other => panic!("{other:?}"),
        })
        .collect();
    let limited = session.send(text("again"), turn("t2")).unwrap().await;

    assert!(exhausted.is_err());
    assert_eq!(
        delays
            .iter()
            .map(|(attempt, _)| *attempt)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    for (attempt, delay) in delays {
        assert!(
            delay < 1_000 << (attempt - 1),
            "attempt {attempt} waited {delay} ms"
        );
    }
    // The vendor's wait outlasts the ceiling: terminal at once.
    assert_eq!(limited.unwrap_err().code.as_deref(), Some("rate_limit"));
    assert_eq!(reports.borrow().len(), 3);
    assert_eq!(provider.requests().len(), 5);
    assert_eq!(
        kinds(&session.transcript().blocks),
        ["user", "error", "user", "error"]
    );
}

#[tokio::test(flavor = "local")]
async fn resume_after_a_failure_that_followed_a_tool_continues_after_its_result() {
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![
            event::tool_call("call-1", "once", json!({})),
            event::response(1, 1),
        ]),
        Turn::Events(vec![
            ProviderEvent::ThinkingStart,
            event::thinking("hmm"),
            failure(None, None),
        ]),
        Turn::Events(vec![event::text("carried on"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let runs = Rc::new(RefCell::new(0));
    let once = tool("once", {
        let runs = runs.clone();
        move |_| {
            *runs.borrow_mut() += 1;
            Box::pin(async { Ok(output("did it")) })
        }
    });
    let session = start(&provider, vec![once], &store, SessionConfig::default()).await;
    assert!(
        session
            .send(text("do it once"), turn("t1"))
            .unwrap()
            .await
            .is_err()
    );
    assert_eq!(
        kinds(&session.transcript().blocks),
        [
            "user",
            "tool_call:completed",
            "response",
            "thinking",
            "error"
        ]
    );

    session.resume().unwrap().await.unwrap();

    assert_eq!(*runs.borrow(), 1);
    assert_eq!(
        kinds(&session.transcript().blocks),
        [
            "user",
            "tool_call:completed",
            "response",
            "resume",
            "text",
            "response"
        ]
    );
    let request = &provider.requests()[2];
    assert_eq!(
        item_kinds(&request.items),
        ["user_message", "tool_use", "tool_result", "user_message"]
    );
    assert_eq!(
        request.items[3],
        InferenceItem::UserMessage {
            content: text(RESUME_TEXT),
        }
    );
}

#[tokio::test(flavor = "local")]
async fn resume_of_a_turn_that_produced_nothing_reruns_it_without_a_resume_block() {
    let provider = ScriptedRuntime::new([
        Turn::Events(vec![failure(None, None)]),
        Turn::Events(vec![event::text("second try"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    assert!(session.send(text("go"), turn("t1")).unwrap().await.is_err());

    session.resume().unwrap().await.unwrap();

    let blocks = session.transcript().blocks;
    assert_eq!(kinds(&blocks), ["user", "text", "response"]);
    let requests = provider.requests();
    assert_eq!(requests[1].items, requests[0].items);
    // The rerun continues the turn it reran.
    assert_eq!(requests[1].turn_id, "t1");
}

#[tokio::test(flavor = "local")]
async fn retry_reruns_the_last_input_turn_with_its_steers_as_one_replacement() {
    let (answer, release) = gated_turn(vec![event::text("first answer"), event::response(1, 1)]);
    let provider = ScriptedRuntime::new([
        answer,
        Turn::Events(vec![event::text("after the steer"), event::response(1, 1)]),
        Turn::Events(vec![
            event::text("a different answer"),
            event::response(1, 1),
        ]),
    ]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    let running = session.send(text("question"), turn("t1")).unwrap();
    until(|| provider.requests().len() == 1).await;
    session
        .steer(text("be brief"), BlockId::try_from("s1").unwrap())
        .unwrap();
    let _ = release.send(());
    running.await.unwrap();
    let patches = Rc::new(RefCell::new(Vec::new()));
    let _listener = session.subscribe({
        let patches = patches.clone();
        move |event| {
            if let SessionEvent::TranscriptChanged { patches: batch, .. } = event {
                patches.borrow_mut().extend(batch.iter().cloned());
            }
        }
    });

    session.retry().unwrap().await.unwrap();

    let blocks = session.transcript().blocks;
    assert_eq!(kinds(&blocks), ["user", "steer", "text", "response"]);
    assert!(
        matches!(&blocks[1], Block::Steer(SteerBlock { turn_id, .. }) if turn_id == &turn("t1"))
    );
    // The rewrite is published as one replacement.
    assert!(matches!(
        patches.borrow().first(),
        Some(TranscriptPatch::Replace { value }) if kinds(value) == ["user", "steer"]
    ));
    let requests = provider.requests();
    assert_eq!(
        item_kinds(&requests[2].items),
        ["user_message", "user_steer"]
    );
    assert_eq!(requests[2].turn_id, "t1");
    let checkpoint = store.checkpoint(&root()).unwrap();
    assert_eq!(
        kinds(&checkpoint.transcript),
        ["user", "steer", "text", "response"]
    );
}

#[tokio::test(flavor = "local")]
async fn resume_after_a_stop_marks_the_stop_resumed_and_continues_the_turn() {
    let provider = ScriptedRuntime::new([
        Turn::pending(),
        Turn::Events(vec![event::text("continued"), event::response(1, 1)]),
    ]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    let running = session.send(text("go"), turn("t1")).unwrap();
    until(|| provider.requests().len() == 1).await;
    session.abort().await;
    running.await.unwrap();

    session.resume().unwrap().await.unwrap();

    let blocks = session.transcript().blocks;
    assert_eq!(
        kinds(&blocks),
        ["user", "abort", "resume", "text", "response"]
    );
    assert!(matches!(&blocks[1], Block::Abort(abort) if abort.is_resumed));
}

#[tokio::test(flavor = "local")]
async fn a_stop_while_resume_saves_its_unwind_leaves_the_unwind_and_one_marker() {
    let provider = ScriptedRuntime::new([Turn::Events(vec![
        event::text("partial"),
        failure(None, None),
    ])]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    assert!(session.send(text("go"), turn("t1")).unwrap().await.is_err());
    session.flush().await.unwrap();
    let gate = store.hold_saves();

    let resuming = session.resume().unwrap();
    until(|| gate.waiting() == 1).await;
    let stopping = session.abort();
    tokio::pin!(stopping);
    // The save that started finishes; the stop is recorded after it.
    assert!(futures_util::poll!(stopping.as_mut()).is_pending());
    gate.release();
    let stopped = stopping.await;

    assert_eq!(stopped.target, Some(AbortTarget::ActiveTurn));
    assert_eq!(resuming.await, Ok(ActionEnd::Aborted));
    assert_eq!(
        kinds(&session.transcript().blocks),
        ["user", "text", "abort"]
    );
    assert_eq!(provider.requests().len(), 1);
}
