//! The live output of commands (`runtime.md` § Live output) at a tree's
//! connections: what a command prints while no page is attached, and what
//! waited when the last page left, never reaches the next page as new
//! output; that page finds the command's current view in its handshake, and
//! only output after it attached arrives on its own. The command runs in a
//! scripted shell environment that prints when the test says, on Tokio's
//! paused clock.

use std::{cell::RefCell, rc::Rc, sync::Arc, time::Duration};

use bytes::Bytes;
use demi_agent::{
    AgentServer, ServerConfig, ServerDeps,
    testing::{ScriptedProviders, TestClient},
};
use demi_agent_protocol::{ServerFrame, ShellStatus};
use demi_agent_store::{AgentTreeStore, testing::MemoryTreeStore};
use demi_agent_tools::{AgentHarness, EnvironmentScope, PromptContext, ShellEnvironmentFactory};
use demi_agent_transcript::testing::SequentialIds;
use demi_core::{CommandId, NodeId, ShellId, StreamKind};
use demi_provider::testing::{FixedClock, ScriptedRuntime, Turn, event};
use demi_shell::{
    CommandRecord, CommandSet, CommandStatus, DEFAULT_OUTPUT_LIMIT_BYTES, ExecRequest, Host,
    HostError, HostFs, HostIdentity, HostKey, HostProcess, PageFeed, PageView, ShellEnvironment,
    ShellError, WholeOutput,
};
use futures_util::future::LocalBoxFuture;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::support::{conversation, is_idle, is_pending_steers, open, send};

/// How often a tree sends its commands' new output, at most.
const INTERVAL: Duration = Duration::from_millis(250);

/// The Host the scripted environment runs on; nothing reaches its files or
/// processes.
#[derive(Debug)]
struct ScriptedHost;

impl Host for ScriptedHost {
    fn key(&self) -> HostKey {
        HostKey::new("scripted")
    }

    fn default_cwd(&self) -> &str {
        "/workspace"
    }

    fn identity(&self) -> HostIdentity {
        HostIdentity {
            uid: 1000,
            gid: 1000,
            hostname: "scripted".into(),
            home_dir: "/home/demi".into(),
        }
    }

    fn fs(&self) -> &dyn HostFs {
        unreachable!("the scripted environment reads no file")
    }

    fn process(&self) -> &dyn HostProcess {
        unreachable!("the scripted environment starts no process")
    }
}

/// A harness whose shell tools reach the scripted Host.
struct LiveHarness;

impl AgentHarness for LiveHarness {
    type Host = ScriptedHost;

    fn name(&self) -> &str {
        "live"
    }

    async fn host(&self, _context: PromptContext<'_>) -> Result<Rc<ScriptedHost>, HostError> {
        Ok(Rc::new(ScriptedHost))
    }

    fn commands(&self) -> Rc<CommandSet> {
        Rc::new(CommandSet::new())
    }

    async fn system_prompt(&self, _context: PromptContext<'_>, _commands: &str) -> String {
        "system prompt".to_owned()
    }
}

/// One node's environment: its one command runs until the test ends, and
/// prints what the test hands it, which it reports to the node's feed.
struct ScriptedShell {
    feed: Rc<dyn PageFeed>,
    command: RefCell<Option<Rc<RefCell<CommandRecord>>>>,
}

impl ScriptedShell {
    fn record(&self, command: &CommandId) -> Result<Rc<RefCell<CommandRecord>>, ShellError> {
        self.command
            .borrow()
            .clone()
            .filter(|record| record.borrow().command_id() == command)
            .ok_or_else(|| ShellError::UnknownCommand(command.clone()))
    }

    /// The running command prints `text`.
    fn print(&self, text: &str) {
        let record = self.command.borrow().clone().expect("the command started");
        record.borrow_mut().append_output(StreamKind::Stdout, text);
        self.feed.changed(&record);
    }
}

impl ShellEnvironment for ScriptedShell {
    fn exec(
        &self,
        request: ExecRequest,
        _cancel: CancellationToken,
    ) -> LocalBoxFuture<'_, Result<CommandStatus, ShellError>> {
        let record = Rc::new(RefCell::new(CommandRecord::new(
            ShellId::try_from("1").unwrap(),
            CommandId::try_from("1").unwrap(),
            request.tool_use_id,
        )));
        self.command.replace(Some(record.clone()));
        self.feed.changed(&record);
        let status = record.borrow_mut().status(DEFAULT_OUTPUT_LIMIT_BYTES, None);
        Box::pin(async move { Ok(status) })
    }

    fn status(&self, command: &CommandId) -> Result<CommandStatus, ShellError> {
        let record = self.record(command)?;
        let status = record.borrow_mut().status(DEFAULT_OUTPUT_LIMIT_BYTES, None);
        Ok(status)
    }

    fn read_output<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Result<WholeOutput, ShellError>> {
        Box::pin(async move { Err(ShellError::UnknownCommand(command.clone())) })
    }

    fn write<'a>(
        &'a self,
        command: &'a CommandId,
        _stdin: Bytes,
    ) -> LocalBoxFuture<'a, Result<(), ShellError>> {
        Box::pin(async move { Err(ShellError::NotRunning(command.clone())) })
    }

    fn abort<'a>(&'a self, command: &'a CommandId) -> LocalBoxFuture<'a, Result<(), ShellError>> {
        Box::pin(async move { Err(ShellError::NotRunning(command.clone())) })
    }

    fn page_views(&self) -> Vec<PageView> {
        let command = self.command.borrow();
        command
            .iter()
            .map(|record| record.borrow().page_view())
            .collect()
    }

    fn release_command<'a>(&'a self, _command: &'a CommandId) -> LocalBoxFuture<'a, bool> {
        Box::pin(async { false })
    }

    fn dispose_shell<'a>(&'a self, _shell: &'a ShellId) -> LocalBoxFuture<'a, bool> {
        Box::pin(async { false })
    }

    fn dispose_all(&self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn owns_shell(&self, shell: &ShellId) -> bool {
        let command = self.command.borrow();
        command
            .as_ref()
            .is_some_and(|record| record.borrow().shell_id() == shell)
    }

    fn owns_command(&self, command: &CommandId) -> bool {
        self.record(command).is_ok()
    }
}

/// Makes the root's environment and keeps it for the test to print through.
#[derive(Default)]
struct ScriptedShells(RefCell<Option<Rc<ScriptedShell>>>);

impl ShellEnvironmentFactory<ScriptedHost> for ScriptedShells {
    fn create<'a>(
        &'a self,
        scope: EnvironmentScope<'a>,
        _host: Rc<ScriptedHost>,
    ) -> LocalBoxFuture<'a, Result<Rc<dyn ShellEnvironment>, HostError>> {
        let shell = Rc::new(ScriptedShell {
            feed: scope.feed.clone(),
            command: RefCell::default(),
        });
        self.0.replace(Some(shell.clone()));
        Box::pin(async move { Ok(shell as Rc<dyn ShellEnvironment>) })
    }
}

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

#[tokio::test(flavor = "local", start_paused = true)]
async fn output_that_waited_or_came_while_no_page_was_attached_never_reaches_the_next_page() {
    let script = ScriptedRuntime::new([
        Turn::Events(vec![event::tool_call(
            "call-1",
            "shell_exec",
            json!({"script": "serve", "timeoutMs": 200}),
        )]),
        Turn::Events(vec![event::text("serving"), event::response(1, 1)]),
    ]);
    let providers = Rc::new(ScriptedProviders::default());
    providers.provide("stub", &script);
    let shells = Rc::new(ScriptedShells::default());
    let store = MemoryTreeStore::new();
    let server = AgentServer::new(ServerDeps {
        harness: Rc::new(LiveHarness),
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
