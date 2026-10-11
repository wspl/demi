//! The live output of commands (`runtime.md` § Live output) at a tree's
//! connections: what a command prints while no page is attached, and what
//! waited when the last page left, never reaches the next page as new
//! output; that page finds the command's current view in its handshake, and
//! only output after it attached arrives on its own. The command runs in a
//! scripted shell environment ([`ScriptedShells`]) that prints when the test
//! says, on Tokio's paused clock.

use std::{rc::Rc, sync::Arc, time::Duration};

use demi_agent_server::{
    AgentServer, ServerConfig, ServerDeps,
    testing::{ScriptedProviders, TestClient},
};
use demi_agent_store::{AgentTreeStore, testing::MemoryTreeStore};
use demi_agent_tools::{SubagentSettings, Toolset};
use demi_agent_transcript::testing::SequentialIds;
use demi_conversation_socket_protocol::{ServerFrame, ShellStatus};
use demi_host_interface::CommandSet;
use demi_provider_common::testing::{FixedClock, ScriptedRuntime, Turn, event};
use demi_shared_types::NodeId;
use serde_json::json;

use crate::support::{
    LiveHosts, ScriptedShell, ScriptedShells, conversation, is_idle, is_pending_steers, open, send,
};

/// How often a tree sends its commands' new output, at most.
const INTERVAL: Duration = Duration::from_millis(250);

/// Lets the tree's sending task run.
async fn settle() {
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
}

/// The tail of each command view among `frames`.
fn tails(frames: &[ServerFrame]) -> Vec<String> {
    frames
        .iter()
        .filter_map(|frame| match frame {
            ServerFrame::ShellOutput { status, .. } => {
                assert!(
                    matches!(**status, ShellStatus::Running { .. }),
                    "{status:?}"
                );
                Some(status.command().tail.clone())
            }
            _ => None,
        })
        .collect()
}

/// A server whose conversation ran a turn that started a command which goes
/// on running, with the page that sent it still attached.
async fn serving() -> (
    Rc<AgentServer<LiveHosts>>,
    Rc<ScriptedShell>,
    TestClient<LiveHosts>,
    ScriptedRuntime,
) {
    let script = ScriptedRuntime::new([
        Turn::Events(vec![event::tool_call(
            "call-1",
            "shell",
            json!({"script": "serve", "description": "Start the server", "intervalMs": null}),
        )]),
        Turn::Events(vec![event::text("serving"), event::response(1, 1)]),
        // The command's end, reported.
        Turn::Events(vec![event::text("the server ended"), event::response(1, 1)]),
    ]);
    let providers = Rc::new(ScriptedProviders::default());
    providers.provide("stub", &script);
    let shells = Rc::new(ScriptedShells::default());
    let store = MemoryTreeStore::new();
    let server = AgentServer::new(ServerDeps {
        toolsets: Rc::new(Toolset {
            commands: Rc::new(CommandSet::new()),
            revision: Rc::from("none"),
        }),
        subagents: Rc::new(SubagentSettings {
            enabled: true,
            max_live: None,
            max_depth: None,
            profiles: Vec::new(),
        }),
        instructions: Rc::from("system prompt"),
        guide: Rc::from("harness guide"),
        hosts: Rc::new(LiveHosts),
        context: Rc::new([]),
        providers,
        shells: shells.clone(),
        stores: Rc::new(move |_: &NodeId| store.clone() as Rc<dyn AgentTreeStore>),
        clock: Arc::new(FixedClock("2026-09-24T12:00:00.000Z".parse().unwrap())),
        ids: Rc::new(SequentialIds::new("id")),
        config: ServerConfig::default(),
        status_changed: Rc::new(|_| {}),
    });
    let mut first = TestClient::connect(&server, &conversation(), "/workspace");
    first.send(open()).await;
    first.next_until(is_pending_steers).await;
    first.send(send("m1", "Serve.")).await;
    first.next_until(is_idle).await;
    let shell = shells
        .0
        .borrow()
        .clone()
        .expect("the call made the environment");
    (server, shell, first, script)
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn output_that_waited_or_came_while_no_page_was_attached_never_reaches_the_next_page() {
    let (server, shell, mut first, _script) = serving().await;

    // Output while a page is attached goes to it after the interval.
    shell.print("one\n");
    tokio::time::advance(INTERVAL).await;
    settle().await;
    assert_eq!(
        tails(&first.received()).last().map(String::as_str),
        Some("one\n")
    );

    // New output waits for the next interval; the page leaves before it goes,
    // and the command prints while no page is attached.
    shell.print("two\n");
    drop(first);
    settle().await;
    shell.print("three\n");

    // The next page finds the command as it is now in its handshake, and
    // nothing of what waited or came while no page was attached follows.
    let mut second = TestClient::connect(&server, &conversation(), "/workspace");
    second.send(open()).await;
    let handshake = second.received();
    assert_eq!(tails(&handshake), ["one\ntwo\nthree\n"], "{handshake:?}");
    tokio::time::advance(INTERVAL * 2).await;
    settle().await;
    assert_eq!(tails(&second.received()), Vec::<String>::new());

    // Output after it attached arrives.
    shell.print("four\n");
    tokio::time::advance(INTERVAL).await;
    settle().await;
    assert_eq!(tails(&second.received()), ["one\ntwo\nthree\nfour\n"]);
}

/// A command that runs is the conversation's work (`resource-lifecycle.md`
/// § Runtime), so a tree no page is attached to stays live while one of its
/// commands runs, and its idle time starts once the last one ended.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_detached_tree_stays_live_while_its_command_runs_and_goes_after_its_idle_time() {
    let (server, shell, first, _script) = serving().await;
    let idle = ServerConfig::default().idle_tree;
    drop(first);

    tokio::time::sleep(idle * 2).await;
    settle().await;
    assert!(
        server.tree(&conversation()).is_some(),
        "the command still runs"
    );

    shell.end();
    tokio::time::sleep(idle - Duration::from_secs(1)).await;
    settle().await;
    assert!(server.tree(&conversation()).is_some());
    tokio::time::sleep(Duration::from_secs(2)).await;
    settle().await;
    assert!(server.tree(&conversation()).is_none());
}

/// A command that outlives its turn is the conversation's work, but it does
/// not make the conversation running (`web-api.md` § Sidebar mutations and
/// read state): once the turn ended the tree no longer works, while the
/// command keeps the tree live.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_command_that_outlives_its_turn_does_not_keep_the_tree_working() {
    let (server, _shell, _first, _script) = serving().await;
    let tree = server.tree(&conversation()).expect("the tree is live");
    assert!(!tree.works());
    assert!(!tree.is_quiescent(), "the command still runs");
}
