//! A command whose start waits for its Host, a paired device whose runner is
//! away (`crates/backend-host-access/src/access.rs`).
//!
//! The tests cover:
//!
//! - It sends no silence report while it waits.
//! - A turn that ends leaves it waiting, and the user's Stop of any later turn
//!   ends it.
//!
//! The commands run in scripted shell environments ([`ScriptedShells`]) that
//! the test marks waiting, on Tokio's paused clock.

use std::{cell::Cell, rc::Rc, time::Duration};

use demi_agent_session::SILENCE_LIMIT;
use demi_agent_server::testing::{ScriptedProviders, TestClient};
use demi_agent_store::testing::MemoryTreeStore;
use demi_conversation_socket_protocol::ClientFrame;
use demi_host_interface::{PageState, ShellEnvironment};
use demi_provider_common::{
    ProviderEvent,
    testing::{ScriptedRuntime, Turn, event},
};
use serde_json::json;

use crate::support::{
        ScriptedShell, ScriptedShells, conversation, is_request_output, request_text, scripted_host_server_telling, send, until,
    };

/// How long a call watches its command before it returns.
const WINDOW: Duration = Duration::from_secs(30);

/// The model's shell call `id` running `script`.
fn shell(id: &str, script: &str) -> ProviderEvent {
    event::tool_call(id, "shell", json!({"script": script, "description": script}))
}

/// The model's closing words.
fn said(text: &str) -> Turn {
    Turn::Events(vec![event::text(text), event::response(1, 1)])
}

/// A page on a server whose provider answers with `script`, with the
/// scripted shells its nodes' commands run in and how many times the
/// product was told of a change of the tree's work.
async fn serving(
    script: &ScriptedRuntime,
) -> (Rc<demi_agent_server::AgentServer<crate::support::LiveHosts>>, Rc<ScriptedShells>, TestClient<crate::support::LiveHosts>, Rc<Cell<usize>>) {
    let providers = Rc::new(ScriptedProviders::default());
    providers.provide("stub", script);
    let shells = Rc::new(ScriptedShells::default());
    let told = Rc::new(Cell::new(0));
    let server = scripted_host_server_telling(providers, shells.clone(), MemoryTreeStore::new(), Rc::default(), {
        let told = told.clone();
        Rc::new(move |_| told.set(told.get() + 1))
    });
    let mut page = TestClient::connect(&server, &conversation());
    page.open().await;
    page.next_until(is_request_output).await;
    (server, shells, page, told)
}

/// The environment that holds `count` commands; a call may make an
/// environment it does not use.
fn holding(shells: &ScriptedShells, count: usize) -> Option<Rc<ScriptedShell>> {
    shells
        .0
        .borrow()
        .iter()
        .find(|shell| shell.page_views().len() == count)
        .cloned()
}

/// The state the pages show of the root's command `command`.
fn state_of(shells: &ScriptedShells, command: &str) -> PageState {
    holding(shells, 1)
        .or_else(|| holding(shells, 2))
        .expect("the command started")
        .page_views()
        .into_iter()
        .find(|view| view.command_id.as_str() == command)
        .map(|view| view.state)
        .expect("the command started")
}

// Half an hour of paused time: a few milliseconds.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_command_waiting_for_its_device_sends_no_silence_report_until_it_runs() {
    let script = ScriptedRuntime::new([
        Turn::Events(vec![shell("call-1", "test"), event::response(1, 1)]),
        said("waiting"),
        said("noted"),
    ]);
    let (_server, shells, mut page, _told) = serving(&script).await;
    page.send(send("m1", "Test.")).await;
    until(|| holding(&shells, 1).is_some()).await;
    let shell = holding(&shells, 1).unwrap();
    shell.wait_for("1", Some("MacBook Pro"));
    // The call returns at its window, its command still waiting.
    tokio::time::sleep(WINDOW).await;
    page.until_idle().await;
    assert_eq!(script.requests().len(), 2);

    // A silence limit and a half pass while it waits: no report.
    tokio::time::sleep(SILENCE_LIMIT * 3 / 2).await;
    assert_eq!(script.requests().len(), 2);

    // Once its device is back it runs, and its silence is reported at the
    // next limit.
    shell.wait_for("1", None);
    tokio::time::sleep(SILENCE_LIMIT).await;
    until(|| script.requests().len() == 3).await;
    let report = request_text(&script.requests()[2]);
    assert!(report.contains("Command 1 (test) is still running"), "{report}");
}

// Thirty seconds of paused time: a few milliseconds.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_turn_that_ends_leaves_its_command_waiting_and_a_later_turns_stop_ends_it() {
    let script = ScriptedRuntime::new([
        Turn::Events(vec![shell("call-1", "test"), event::response(1, 1)]),
        said("waiting"),
        // The next turn's request runs until the Stop.
        Turn::pending(),
    ]);
    let (_server, shells, mut page, _told) = serving(&script).await;
    page.send(send("m1", "Test.")).await;
    until(|| holding(&shells, 1).is_some()).await;
    holding(&shells, 1).unwrap().wait_for("1", Some("MacBook Pro"));
    tokio::time::sleep(WINDOW).await;
    page.until_idle().await;
    // The turn ended; its command, a background command not yet started,
    // waits on.
    assert_eq!(state_of(&shells, "1"), PageState::Waiting { host: "MacBook Pro".into() });

    page.send(send("m2", "Anything else?")).await;
    until(|| script.requests().len() == 3).await;
    page.send(ClientFrame::Abort {}).await;
    page.until_idle().await;
    assert_eq!(state_of(&shells, "1"), PageState::Aborted, "the Stop ends every waiting command");
}
