//! What a root's failed turn does with the input that arrives after it.

use super::*;

fn answer(text: &str) -> Turn {
    Turn::Events(vec![event::text(text), event::response(0, 0)])
}

fn used_up() -> Turn {
    Turn::Events(vec![event::error(
        "You've hit your usage limit",
        Some(ErrorCode::UsageLimit),
    )])
}

/// Whether the request `request` shows the model the text `read`.
fn asks(request: &demi_provider_common::InferenceRequest, read: &str) -> bool {
    // The items' debug form escapes the text as a string literal does.
    let escaped = format!("{read:?}");
    format!("{:?}", request.items).contains(&escaped[1..escaped.len() - 1])
}

/// A root's failed turn holds a message from another agent: it is written
/// into the failed turn and starts nothing. Once the failure has passed, the
/// message held before still starts nothing by itself, while the next one
/// opens a turn that reads both. A later failure holds again.
// One session and four scripted requests: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_message_after_a_failure_that_passed_opens_a_turn() {
    let provider = ScriptedRuntime::new([used_up(), answer("read both"), used_up(), answer("unexpected")]);
    let store = MemoryTreeStore::new();
    let session = start(&provider, Vec::new(), &store, SessionConfig::default()).await;
    session.send(text("build it"), turn("t1")).unwrap().await.unwrap_err();
    let failure = session.failed_turn().expect("the turn failed").id;

    session.accept_agent_message(agent_message("one")).await.unwrap();
    session.settled().await;
    let blocks = session.transcript().blocks;
    assert_eq!(kinds(&blocks[blocks.len() - 2..]), ["error", "agent_message"]);
    assert_eq!(provider.requests().len(), 1, "the failed turn holds the message");

    session.failure_passed(&BlockId::try_from("another-failure").unwrap());
    session.accept_agent_message(agent_message("two")).await.unwrap();
    session.settled().await;
    assert_eq!(provider.requests().len(), 1, "another failure passing ends no hold");

    session.failure_passed(&failure);
    session.settled().await;
    assert_eq!(provider.requests().len(), 1, "what the hold wrote starts nothing by itself");

    session.accept_agent_message(agent_message("three")).await.unwrap();
    until(|| provider.requests().len() == 2).await;
    session.settled().await;
    let requests = provider.requests();
    for read in ["Result one", "Result two", "Result three"] {
        assert!(asks(&requests[1], read), "{read} in {:?}", requests[1].items);
    }
    assert!(session.failed_turn().is_none());

    session.accept_agent_message(agent_message("four")).await.unwrap();
    until(|| provider.requests().len() == 3).await;
    session.settled().await;
    assert!(session.failed_turn().is_some());
    session.accept_agent_message(agent_message("five")).await.unwrap();
    session.settled().await;
    assert_eq!(provider.requests().len(), 3, "the new failure holds again");
}
