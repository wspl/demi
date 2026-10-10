//! A conversation whose Host goes offline (`sessions-and-targets.md` § Host
//! operations, § Switch the primary target; `product.md` § Recovering an
//! unfinished turn): a `shell` call on an offline primary device waits for
//! its runner, its row says so, and it runs once the runner is back; Stop
//! ends the wait as it ends any turn; a move meanwhile ends the waiting call
//! as not run, tells the agent, and the turn goes on on the new Host; an
//! offline attached device fails at once with its offline error; and a tree
//! that works otherwise still refuses the move. The model is an Anthropic
//! endpoint the test scripts, or a scripted family for a tree with a child;
//! the devices are real runners.

use std::sync::Arc;

use demi_conversation_socket_protocol::ServerFrame;
use demi_provider_common::testing::MockVendor;
use demi_shared_types::SessionPhase;
use demi_web_api_protocol::error::ErrorCode;
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{Socket, anthropic_at, create, send};
use crate::subagents::{self, Scripts, tree_on};
use crate::support::{Harness, Paired, Session, TestBackend};
use crate::work::{Driven, say, shell, switch};

const FIRST: &str = "7e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a07";

/// A conversation in `paired`'s home, opened.
async fn on<'a>(
    backend: &TestBackend,
    master: &Session,
    vendor: &'a MockVendor,
    paired: &Paired,
) -> Driven<'a> {
    let provider = anthropic_at(backend, master, vendor, "/offline").await;
    create(backend, master, FIRST).await;
    let home = paired.runner.home_dir().to_owned();
    switch(backend, master, FIRST, paired, &home).await;
    Driven::open(backend, master, vendor, FIRST, &provider, "/offline").await
}

/// Stops `paired`'s runner and waits until the backend shows it offline.
async fn offline(backend: &TestBackend, master: &Session, paired: &mut Paired) {
    paired.runner.stop().await;
    backend.until_online(master, paired.id(), false).await;
}

/// Waits until the page is told that the root's call `call` waits for the
/// Host `host`.
async fn until_waiting(socket: &mut Socket, call: &str, host: &str) {
    socket
        .until(|frame| {
            matches!(frame, ServerFrame::WaitingCalls { subagent_id: None, waiting_calls }
                if waiting_calls.iter().any(|waiting| waiting.tool_use_id == call && waiting.host == host))
        })
        .await;
}

/// Waits for the idle phase of a turn whose running phase was read already.
async fn until_idle(socket: &mut Socket) {
    socket
        .until(|frame| matches!(frame, ServerFrame::Phase { phase: SessionPhase::Idle }))
        .await;
}

#[tokio::test]
async fn a_shell_call_on_an_offline_primary_device_waits_for_its_runner_and_runs_once_it_is_back() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut alpha = backend.pair(&master, "alpha").await;
    let mut work = on(&backend, &master, &vendor, &alpha).await;
    offline(&backend, &master, &mut alpha).await;

    let before = work
        .start(vec![shell("t1", "echo back", 10_000), say("done")])
        .await;
    until_waiting(&mut work.socket, "t1", "alpha").await;
    alpha.runner.start_again();
    until_idle(&mut work.socket).await;

    // The model never learns that the Host was away: it reads the command's
    // ordinary result.
    let turn = work.observe(before);
    assert!(
        turn.received[0].contains("exitCode: 0") && turn.received[0].contains("back"),
        "{}",
        turn.received[0]
    );
    backend.close().await;
}

#[tokio::test]
async fn a_stop_ends_a_shell_call_that_waits_for_the_offline_primary_device() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut alpha = backend.pair(&master, "alpha").await;
    let mut work = on(&backend, &master, &vendor, &alpha).await;
    offline(&backend, &master, &mut alpha).await;

    work.start(vec![shell("t1", "echo back", 10_000)]).await;
    until_waiting(&mut work.socket, "t1", "alpha").await;
    work.socket.stop().await;

    // The next turn's request carries the stopped call's result.
    let next = work.turn(vec![say("stopped")]).await;
    assert_eq!(next.received, ["Tool call aborted: the user stopped the turn."]);
    backend.close().await;
}

#[tokio::test]
async fn a_move_while_a_call_waits_ends_it_as_not_run_tells_the_agent_and_the_turn_goes_on_on_the_new_host() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut alpha = backend.pair(&master, "alpha").await;
    let beta = backend.pair(&master, "beta").await;
    let mut work = on(&backend, &master, &vendor, &alpha).await;
    offline(&backend, &master, &mut alpha).await;

    let before = work
        .start(vec![
            shell("t1", "pwd", 10_000),
            shell("t2", "pwd", 10_000),
            say("moved on"),
        ])
        .await;
    until_waiting(&mut work.socket, "t1", "alpha").await;
    // A move without notifyAgent: a waiting turn is told all the same.
    let beta_home = beta.runner.home_dir().to_owned();
    switch(&backend, &master, FIRST, &beta, &beta_home).await;
    until_idle(&mut work.socket).await;

    let turn = work.observe(before);
    assert_eq!(
        turn.received[0],
        "Tool call not run: the user moved this conversation from alpha to beta while this call waited for alpha, which was offline. Run it again there if it is still needed."
    );
    let next = turn.requests[1]["messages"].to_string();
    assert!(
        next.contains("The user moved this conversation from alpha (") && next.contains(" to beta ("),
        "{next}"
    );
    assert!(next.contains("[Execution context "), "{next}");
    assert!(
        turn.received[1].contains(beta_home.to_str().unwrap()),
        "{}",
        turn.received[1]
    );
    backend.close().await;
}

#[tokio::test]
async fn an_offline_attached_device_fails_at_once_with_its_offline_error() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let mut build = backend.pair(&master, "build-box").await;
    let mut work = on(&backend, &master, &vendor, &alpha).await;
    harness.attach(FIRST, build.id(), "build-box");
    offline(&backend, &master, &mut build).await;

    let turn = work
        .turn(vec![
            shell("t1", "demi host shell --host build-box 'echo hi'", 10_000),
            say("noted"),
        ])
        .await;
    assert!(
        turn.received[0].contains("build-box is offline: its runner has been disconnected for ")
            && turn.received[0].contains(", so nothing can run there now; commands already running there are kept for up to 10m and report when it is back."),
        "{}",
        turn.received[0]
    );
    backend.close().await;
}

#[tokio::test]
async fn a_move_is_refused_while_a_child_thinks_beside_a_call_that_waits() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, mut paired, _root, _provider) =
        tree_on(&scripts, Harness::new()).await;
    let conversation = crate::conversations::FIRST;
    let mut socket = Socket::connect(&backend, &master, conversation).await;
    socket.open().await;
    scripts.root(
        conversation,
        vec![
            subagents::shell("t1", "demi agent spawn --description thinker <<< 'Think it over'"),
            subagents::say("spawned"),
        ],
    );
    // The child's request stays open: it is still thinking.
    scripts.child(vec![vec![]]);
    socket.chat("m1", "Spawn a thinker").await;
    offline(&backend, &master, &mut paired).await;

    scripts.root(
        conversation,
        vec![subagents::shell("t2", "echo hi"), subagents::say("after")],
    );
    socket.send(&send("m2", "Go on")).await;
    until_waiting(&mut socket, "t2", "laptop").await;
    let refused = backend
        .patch(
            &format!("/api/conversations/{conversation}"),
            &master,
            json!({ "target": { "kind": "cloud" } }),
        )
        .await;
    assert_eq!(
        refused.refusal(),
        (StatusCode::CONFLICT, ErrorCode::TurnInFlight)
    );
    backend.close().await;
}
