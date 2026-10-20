//! A command whose start waits for its Host, a paired device whose runner
//! is away (`crates/backend-host-access/src/access.rs`): it sends no
//! interval report while it waits, a turn that ends leaves it waiting and
//! the user's Stop of any later turn ends it, and the tree says, when it only waits for the
//! device, that a move stops nothing (§ Switch the primary target). The
//! commands run in scripted shell environments ([`ScriptedShells`]) that the
//! test marks waiting, on Tokio's paused clock.

use std::{cell::Cell, rc::Rc, time::Duration};

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
    ScriptedShell, ScriptedShells, conversation, is_idle, is_pending_steers, open, request_text, scripted_host_server_telling,
    send, until,
};

/// How often the commands report: the least interval a call can ask for.
const INTERVAL: Duration = Duration::from_secs(15);

/// The model's shell call `id` running `script`, which reports every
/// [`INTERVAL`] while it runs.
fn shell(id: &str, script: &str) -> ProviderEvent {
    event::tool_call(id, "shell", json!({"script": script, "description": script, "intervalMs": 15_000}))
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
    page.send(open()).await;
    page.next_until(is_pending_steers).await;
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

// Two parallel calls, then the device's absence and return for each: a
// few milliseconds on the paused clock.
#[tokio::test(flavor = "local", start_paused = true)]
async fn the_tree_only_waits_once_every_call_still_running_waits_for_the_device_and_says_so() {
    let script = ScriptedRuntime::new([
        Turn::Events(vec![shell("call-1", "test"), shell("call-2", "build"), event::response(1, 1)]),
        said("done"),
    ]);
    let (server, shells, page, told) = serving(&script).await;
    page.send(send("m1", "Test and build.")).await;
    until(|| holding(&shells, 2).is_some()).await;
    let tree = server.tree(&conversation()).unwrap();
    let shell = holding(&shells, 2).unwrap();

    // One call waits for the device while the other runs there: the agent
    // works, so a move would stop it.
    shell.wait_for("1", Some("MacBook Pro"));
    until(|| state_of(&shells, "1") == PageState::Waiting { host: "MacBook Pro".into() }).await;
    assert!(!tree.only_waits());

    // Both wait: the tree only waits, and the product hears of it, as it
    // does when one comes back and runs.
    let before = told.get();
    shell.wait_for("2", Some("MacBook Pro"));
    until(|| tree.only_waits() && told.get() > before).await;
    let before = told.get();
    shell.wait_for("1", None);
    until(|| !tree.only_waits() && told.get() > before).await;

    shell.exit("1");
    shell.exit("2");
    let mut page = page;
    page.next_until(is_idle).await;
    assert!(!tree.only_waits(), "a tree at rest does not wait");
}

// Three minutes of paused time: a few milliseconds.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_command_waiting_for_its_device_sends_no_interval_report_until_it_runs() {
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
    // The call returns at its interval, its command still waiting.
    tokio::time::sleep(INTERVAL).await;
    page.next_until(is_idle).await;
    assert_eq!(script.requests().len(), 2);

    // Ten intervals pass while it waits: no report.
    tokio::time::sleep(INTERVAL * 10).await;
    assert_eq!(script.requests().len(), 2);

    // Once its device is back it runs, and reports at its next interval.
    shell.wait_for("1", None);
    tokio::time::sleep(INTERVAL).await;
    until(|| script.requests().len() == 3).await;
    let report = request_text(&script.requests()[2]);
    assert!(report.contains("Command 1 (test) is still running"), "{report}");
}

// Fifteen seconds of paused time: a few milliseconds.
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
    tokio::time::sleep(INTERVAL).await;
    page.next_until(is_idle).await;
    // The turn ended; its command, a background command not yet started,
    // waits on.
    assert_eq!(state_of(&shells, "1"), PageState::Waiting { host: "MacBook Pro".into() });

    page.send(send("m2", "Anything else?")).await;
    until(|| script.requests().len() == 3).await;
    page.send(ClientFrame::Abort {}).await;
    page.next_until(is_idle).await;
    assert_eq!(state_of(&shells, "1"), PageState::Aborted, "the Stop ends every waiting command");
}
