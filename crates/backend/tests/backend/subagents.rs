//! Subagents through a conversation (`subagents.md`, `sessions-and-targets.md`
//! § Host operations, `conversation-fork.md`): a child works on the
//! conversation's Host in its parent's files, keeps its own command storage
//! even for a command it runs through `demi host shell`, and runs on after
//! its spawn command has exited; a Fork taken while a child runs leaves the
//! child with its source. The model is a scripted family that answers each
//! node of a tree from a script of its own; the device is a real runner. No
//! test calls a real model.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

use demi_agent::testing::model_of;
use demi_agent_protocol::JobPhase;
use demi_backend::{FamilyArgs, FamilyCredential, FamilyError, FamilyRegistry, ProviderFamily};
use demi_core::{
    AuthState, Block, ModelSelection, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList,
    RuntimeState, Timestamp,
};
use demi_provider::testing::event;
use demi_provider::{
    Capabilities, CatalogError, InferenceRequest, Provider, ProviderEvent, ProviderRun, ProviderRuntime, RuntimeEnv,
    RuntimeError,
};
use demi_web_api::conversations::ForkAnswer;
use demi_web_api::providers::{CredentialKind, ProviderAnswer};
use futures_util::future::{BoxFuture, LocalBoxFuture};
use futures_util::{StreamExt as _, stream};
use reqwest::StatusCode;
use serde_json::json;

use crate::conversations::{FIRST, SECOND, Socket, create, on_device, transcript};
use crate::support::{Harness, Session, TestBackend};

/// One scripted answer: the events of one request.
type Answer = Vec<ProviderEvent>;

/// The answers of each node: a root's by its session, which is its
/// conversation's id, and each child's in the order the children first ask.
#[derive(Default)]
struct Scripts {
    state: Mutex<ScriptState>,
}

#[derive(Default)]
struct ScriptState {
    roots: HashMap<String, VecDeque<Answer>>,
    /// The scripts of the children that have not asked yet.
    unclaimed: VecDeque<VecDeque<Answer>>,
    /// The scripts of the children that have asked, by session.
    children: HashMap<String, VecDeque<Answer>>,
    /// Every request: its session and what it carried.
    asked: Vec<(String, String)>,
}

impl Scripts {
    fn root(&self, conversation: &str, answers: Vec<Answer>) {
        let mut state = self.state.lock().unwrap();
        state.roots.entry(conversation.to_owned()).or_default().extend(answers);
    }

    fn child(&self, answers: Vec<Answer>) {
        self.state.lock().unwrap().unclaimed.push_back(answers.into());
    }

    /// What the requests of the sessions `keep` accepts carried, in order.
    fn asked(&self, keep: impl Fn(&str) -> bool) -> Vec<String> {
        let state = self.state.lock().unwrap();
        state
            .asked
            .iter()
            .filter(|(session, _)| keep(session))
            .map(|(_, items)| items.clone())
            .collect()
    }

    fn answer(&self, request: &InferenceRequest) -> Answer {
        let mut guard = self.state.lock().unwrap();
        let state = &mut *guard;
        let session = request.session_id.clone();
        state.asked.push((session.clone(), format!("{:?}", request.items)));
        let script = match state.roots.get_mut(&session) {
            Some(script) => script,
            None => {
                let unclaimed = &mut state.unclaimed;
                state.children.entry(session.clone()).or_insert_with(|| {
                    unclaimed
                        .pop_front()
                        .unwrap_or_else(|| panic!("no script for the child {session}"))
                })
            }
        };
        script
            .pop_front()
            .unwrap_or_else(|| panic!("no answer scripted for {session}"))
    }
}

/// The model's shell call `id` running `script`.
fn shell(id: &str, script: &str) -> Answer {
    vec![
        event::tool_call(id, "shell_exec", json!({ "description": id, "script": script, "timeoutMs": 60_000 })),
        event::response(1, 1),
    ]
}

/// The model's closing words.
fn say(text: &str) -> Answer {
    vec![event::text(text), event::response(1, 1)]
}

struct Tree(Arc<Scripts>);

impl ProviderFamily for Tree {
    fn credential(&self) -> CredentialKind {
        CredentialKind::ApiKey
    }

    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
        let FamilyCredential::ApiKey(_) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        Ok(Arc::new(TreeProvider {
            id: args.entry_id,
            scripts: self.0.clone(),
        }))
    }
}

struct TreeProvider {
    id: String,
    scripts: Arc<Scripts>,
}

impl Provider for TreeProvider {
    fn id(&self) -> &str {
        &self.id
    }

    fn display_name(&self) -> &str {
        "Tree"
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn auth_status(&self) -> BoxFuture<'_, AuthState> {
        Box::pin(async { AuthState::Authenticated { account_label: None } })
    }

    fn runtime_state(&self) -> RuntimeState {
        RuntimeState::Ready { message: None }
    }

    fn list_models(&self) -> BoxFuture<'_, Result<ProviderModelList, CatalogError>> {
        Box::pin(async { Err(CatalogError::Unavailable("no directory".into())) })
    }

    fn read_failure(&self, _: &ProviderErrorDiagnostics, _: Timestamp) -> ProviderFailureFacts {
        ProviderFailureFacts { retry_at: None }
    }

    fn runtime(&self, _: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        Ok(Box::new(TreeRuntime(self.scripts.clone())))
    }
}

struct TreeRuntime(Arc<Scripts>);

impl ProviderRuntime for TreeRuntime {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        let answer = self.0.answer(&request);
        stream::iter(answer)
            .take_until(request.cancel.cancelled_owned())
            .boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(Self(self.0.clone()))
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

/// A backend whose master has an entry of the scripted family, the model of
/// that entry, and the conversation `FIRST` on a paired device's `work`
/// directory; the harness holds the backend's data and the device.
async fn tree(scripts: &Arc<Scripts>) -> (Harness, TestBackend, Session, ModelSelection, crate::support::Paired, String) {
    let harness = Harness::new().with_families(FamilyRegistry::builtin().with("tree", Tree(scripts.clone())));
    let (backend, master) = harness.start_set_up().await;
    let created = backend
        .post(
            "/api/providers",
            Some(&master),
            json!({ "source": "custom", "providerType": "tree", "label": "Tree", "apiKey": "k" }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    let provider = created.json::<ProviderAnswer>().provider.id.as_str().to_owned();
    create(&backend, &master, FIRST).await;
    let (paired, root) = on_device(&harness, &backend, &master, FIRST).await;
    (harness, backend, master, model_of(&provider, "m"), paired, root)
}

/// The shell that waits until the file `go` appears where it works.
const WAIT: &str = "until [ -f go ]; do sleep 0.05; done";

#[tokio::test]
async fn a_child_works_in_its_parents_files_keeps_its_own_todos_and_runs_on_after_its_spawn() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, model, _paired, root) = tree(&scripts).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open(&model).await;

    scripts.root(FIRST, vec![
        shell("t1", "printf 'the answer is 42\\n' > notes.md && demi todo add root-only"),
        say("written"),
    ]);
    socket.chat("m1", "Write the notes").await;

    // The child waits until the test lets it go, long after the spawn
    // command exited; then it reads the parent's file, writes one of its
    // own, and adds a todo through a shell on the conversation's device.
    let child = format!(
        "{WAIT}; cat notes.md && printf 'from the child\\n' > reply.md && \
         demi host shell --host laptop \"demi todo add child-only\" && demi todo list"
    );
    scripts.child(vec![shell("c1", &child), say("the file says 42")]);
    scripts.root(FIRST, vec![
        shell("t2", "demi agent spawn --description reader <<< 'Read notes.md and answer'"),
        say("dispatched"),
        // The child's completion wakes the idle parent.
        say("received"),
    ]);
    socket.chat("m2", "Delegate the reading").await;
    std::fs::write(format!("{root}/go"), "").unwrap();
    socket.until_idle().await;

    let children = scripts.asked(|session| session != FIRST);
    let read = children.last().unwrap();
    assert!(read.contains("the answer is 42") && read.contains("child-only"), "{read}");
    assert!(!read.contains("root-only"), "the child's todos are its own: {read}");
    assert_eq!(std::fs::read_to_string(format!("{root}/reply.md")).unwrap(), "from the child\n");

    scripts.root(FIRST, vec![shell("t3", "cat reply.md && demi todo list"), say("checked")]);
    socket.chat("m3", "Check").await;
    let checked = scripts.asked(|session| session == FIRST);
    let checked = &checked[checked.len() - 1];
    assert!(checked.contains("from the child") && checked.contains("root-only"), "{checked}");
    assert!(!checked.contains("child-only"), "the parent's todos are its own: {checked}");

    let history = transcript(&backend, &master, FIRST).await;
    let jobs: Vec<(&str, JobPhase)> = history
        .subagents
        .iter()
        .map(|child| (child.subagent.description.as_str(), child.subagent.phase))
        .collect();
    assert_eq!(jobs, [("reader", JobPhase::Completed)]);
    backend.close().await;
}

#[tokio::test]
async fn a_fork_taken_while_a_child_runs_leaves_the_child_with_its_source() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, model, _paired, root) = tree(&scripts).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open(&model).await;
    scripts.child(vec![shell("c1", WAIT), say("the child's result")]);
    scripts.root(FIRST, vec![
        shell("t1", "demi agent spawn --description worker <<< 'Wait for the file'"),
        say("the child is still working"),
        say("received"),
    ]);
    socket.chat("m1", "Start a worker").await;

    let text = match socket.live().await.iter().rev().find(|block| matches!(block, Block::Text(_))) {
        Some(block) => block.id().clone(),
        None => panic!("the parent answered"),
    };
    let created = backend
        .post(
            &format!("/api/conversations/{FIRST}/fork"),
            Some(&master),
            json!({ "id": SECOND, "blockId": text }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    let forked = created.json::<ForkAnswer>();
    // The Fork keeps the call that spawned the child, and no child.
    let before = transcript(&backend, &master, SECOND).await;
    assert!(before.blocks.iter().any(|block| matches!(block, Block::ToolCall(_))), "{:?}", before.blocks);
    assert!(before.subagents.is_empty(), "{:?}", before.subagents);

    // The child finishes into its source alone.
    std::fs::write(format!("{root}/go"), "").unwrap();
    socket.until_idle().await;
    let source = transcript(&backend, &master, FIRST).await;
    assert_eq!(source.subagents.len(), 1);
    assert_eq!(transcript(&backend, &master, SECOND).await, before);

    // The Fork's tree has no child to list.
    let mut fork = Socket::connect(&backend, &master, SECOND).await;
    fork.open(&forked.model).await;
    scripts.root(SECOND, vec![shell("f1", "demi agent list"), say("an empty tree")]);
    fork.chat("m2", "Who works for you?").await;
    let asked = scripts.asked(|session| session == SECOND);
    let last = &asked[asked.len() - 1];
    let listed = &last[last.find("ToolResult { tool_use_id: \"f1\"").expect("the list's result")..];
    assert!(listed.contains("(root session)") && !listed.contains("worker"), "{listed}");
    backend.close().await;
}
