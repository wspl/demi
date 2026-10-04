//! Subagents through a conversation (`subagents.md`, `sessions-and-targets.md`
//! § Host operations, `conversation-fork.md`): a child works on the
//! conversation's Host in its parent's files, keeps its own identity even
//! for a command it runs through `demi host shell`, and runs on after
//! its spawn command has exited; a Fork taken while a child runs leaves the
//! child with its source; a spawn reads the user's subagent settings and
//! builds its child's model from the catalog; and a Cloud reset holds a
//! conversation whose child's profile infers with a provider on the Cloud.
//! The model is a scripted family that answers each
//! node of a tree from a script of its own; the device is a real runner. No
//! test calls a real model.

use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use demi_backend_providers::llm::families::{
    FamilyArgs, FamilyCredential, FamilyError, ProviderFamily,
};
use demi_conversation_socket_protocol::{JobPhase, ServerFrame, SubagentEvent};
use demi_provider_claude_code::Placement;
use demi_provider_common::testing::event;
use demi_provider_common::{
    Capabilities, CatalogError, InferenceRequest, Provider, ProviderEvent, ProviderRun,
    ProviderRuntime, RequestLimits, RuntimeEnv, RuntimeError,
};
use demi_shared_types::{
    AuthState, Block, ProviderErrorDiagnostics, ProviderFailureFacts, ProviderModelList,
    RuntimeState, Timestamp,
};
use demi_web_api_protocol::providers::{CredentialKind, ProviderAnswer};
use futures_util::future::{BoxFuture, LocalBoxFuture};
use futures_util::{StreamExt as _, stream};
use reqwest::StatusCode;
use serde_json::json;
use tokio::sync::Notify;

use crate::conversations::{FIRST, SECOND, Socket, choose, create, on_device, transcript};
use crate::cloud::reset;
use crate::support::{Harness, Session, TestBackend, eventually};

/// One scripted answer: the events of one request.
pub(crate) type Answer = Vec<ProviderEvent>;

/// The answers of each node: a root's by its session, which is its
/// conversation's id, and each child's in the order the children first ask.
#[derive(Default)]
pub(crate) struct Scripts {
    state: Mutex<ScriptState>,
}

#[derive(Default)]
struct ScriptState {
    roots: HashMap<String, VecDeque<Answer>>,
    /// The scripts of the children that have not asked yet.
    unclaimed: VecDeque<VecDeque<Answer>>,
    /// The scripts of the children that have asked, by session.
    children: HashMap<String, VecDeque<Answer>>,
    /// Every request: its session, and its model, thinking, system prompt
    /// and items.
    asked: Vec<(String, String)>,
}

impl Scripts {
    pub(crate) fn root(&self, conversation: &str, answers: Vec<Answer>) {
        let mut state = self.state.lock().unwrap();
        state
            .roots
            .entry(conversation.to_owned())
            .or_default()
            .extend(answers);
    }

    pub(crate) fn child(&self, answers: Vec<Answer>) {
        self.state
            .lock()
            .unwrap()
            .unclaimed
            .push_back(answers.into());
    }

    /// What the requests of the sessions `keep` accepts carried, in order.
    pub(crate) fn asked(&self, keep: impl Fn(&str) -> bool) -> Vec<String> {
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
        let carried = format!(
            "model={} thinking={:?} system={:?} items={:?}",
            request.model_id, request.thinking, request.system_prompt, request.items
        );
        state.asked.push((session.clone(), carried));
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
pub(crate) fn shell(id: &str, script: &str) -> Answer {
    vec![
        event::tool_call(
            id,
            "shell_exec",
            json!({ "description": id, "script": script, "timeoutMs": 60_000 }),
        ),
        event::response(1, 1),
    ]
}

/// The model's closing words.
pub(crate) fn say(text: &str) -> Answer {
    vec![event::text(text), event::response(1, 1)]
}

struct Tree {
    scripts: Arc<Scripts>,
    /// Whether its provider runs a process on a Host, which the user's Cloud
    /// then is.
    process: bool,
}

impl ProviderFamily for Tree {
    fn credential(&self) -> CredentialKind {
        CredentialKind::ApiKey
    }

    fn provider(&self, args: FamilyArgs) -> Result<Arc<dyn Provider>, FamilyError> {
        let FamilyCredential::ApiKey(_) = args.credential else {
            return Err(FamilyError::WrongCredential);
        };
        Ok(Arc::new(TreeProvider {
            scripts: self.scripts.clone(),
            process: self.process,
        }))
    }

    /// The scripted runtime, wherever the placement would start the process.
    fn process_runtime(
        &self,
        _: FamilyArgs,
        _: Rc<dyn Placement>,
    ) -> Option<Result<Box<dyn ProviderRuntime>, FamilyError>> {
        let runtime: Box<dyn ProviderRuntime> = Box::new(TreeRuntime(self.scripts.clone()));
        self.process.then_some(Ok(runtime))
    }
}

struct TreeProvider {
    scripts: Arc<Scripts>,
    process: bool,
}

impl Provider for TreeProvider {
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            process_host: self.process,
        }
    }

    fn auth_status(&self) -> BoxFuture<'_, AuthState> {
        Box::pin(async {
            AuthState::Authenticated {
                account_label: None,
            }
        })
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

    fn request_limits(&self, _model: &demi_shared_types::Model) -> RequestLimits {
        RequestLimits::default()
    }
}

/// A backend whose master has an entry of the scripted family `tree`, with the
/// model `m`, which lists no effort, and the model `n`, which lists `low`
/// and `high`, and the conversation `FIRST` on `m` and a paired device's
/// `work` directory; the harness holds the backend's data and the device.
/// Answers the entry's id last. The family `tree-process` scripts the same
/// answers from a provider that runs a process on the Cloud.
async fn tree(
    scripts: &Arc<Scripts>,
) -> (
    Harness,
    TestBackend,
    Session,
    crate::support::Paired,
    String,
    String,
) {
    tree_on(scripts, Harness::new()).await
}

/// [`tree`] on `harness`, such as one with skill repositories.
pub(crate) async fn tree_on(
    scripts: &Arc<Scripts>,
    harness: Harness,
) -> (
    Harness,
    TestBackend,
    Session,
    crate::support::Paired,
    String,
    String,
) {
    let tree = |process| Tree {
        scripts: scripts.clone(),
        process,
    };
    let families = demi_backend::families::builtin()
        .with("tree", tree(false))
        .with("tree-process", tree(true));
    let harness = harness.with_families(families);
    let (backend, master) = harness.start_set_up().await;
    let created = backend
        .post(
            "/api/providers",
            Some(&master),
            json!({
                "source": "custom", "providerType": "tree", "label": "Tree", "apiKey": "k",
                "models": [
                    {
                        "id": "m", "displayName": "M", "contextWindow": 100000, "outputLimit": null,
                        "thinkingEfforts": [], "acceptedExtensions": null, "fastTier": null
                    },
                    {
                        "id": "n", "displayName": "N", "contextWindow": 100000, "outputLimit": null,
                        "thinkingEfforts": ["low", "high"], "acceptedExtensions": null,
                        "fastTier": null
                    }
                ]
            }),
        )
        .await;
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    let provider = created
        .json::<ProviderAnswer>()
        .provider
        .id
        .as_str()
        .to_owned();
    create(&backend, &master, FIRST).await;
    choose(&backend, &master, FIRST, &provider, "m").await;
    let (paired, root) = on_device(&harness, &backend, &master, FIRST).await;
    (harness, backend, master, paired, root, provider)
}

/// The shell that waits until the file `go` appears where it works.
pub(crate) const WAIT: &str = "until [ -f go ]; do sleep 0.05; done";

// Several seconds: a parent and its child run five shell jobs on a real device,
// the child's across its parent's turns.
#[tokio::test]
async fn a_child_works_in_its_parents_files_keeps_its_identity_and_runs_on_after_its_spawn() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _paired, root, _provider) = tree(&scripts).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    scripts.root(
        FIRST,
        vec![
            shell("t1", "printf 'the answer is 42\\n' > notes.md"),
            say("written"),
        ],
    );
    socket.chat("m1", "Write the notes").await;

    // The child waits until the test lets it go, long after the spawn
    // command exited; then it reads the parent's file, writes one of its
    // own, and lists the agents through a shell on the conversation's
    // device, which knows it as the caller.
    let child = format!(
        "{WAIT}; cat notes.md && printf 'from the child\\n' > reply.md && \
         demi host shell --host laptop \"demi agent list\""
    );
    scripts.child(vec![shell("c1", &child), say("the file says 42")]);
    scripts.root(
        FIRST,
        vec![
            shell(
                "t2",
                "demi agent spawn --description reader <<< 'Read notes.md and answer'",
            ),
            say("dispatched"),
            // The child's completion wakes the idle parent.
            say("received"),
        ],
    );
    socket.chat("m2", "Delegate the reading").await;
    std::fs::write(format!("{root}/go"), "").unwrap();
    socket.until_idle().await;

    let children = scripts.asked(|session| session != FIRST);
    let read = children.last().unwrap();
    // The tree is the root and the child: the caller is the child.
    assert!(
        read.contains("the answer is 42") && read.contains("← you"),
        "{read}"
    );
    assert!(
        !read.contains("(root session) ← you"),
        "the child is the caller: {read}"
    );
    assert_eq!(
        std::fs::read_to_string(format!("{root}/reply.md")).unwrap(),
        "from the child\n"
    );

    scripts.root(
        FIRST,
        vec![
            shell("t3", "cat reply.md"),
            say("checked"),
        ],
    );
    socket.chat("m3", "Check").await;
    let checked = scripts.asked(|session| session == FIRST);
    let checked = &checked[checked.len() - 1];
    assert!(checked.contains("from the child"), "{checked}");

    let history = transcript(&backend, &master, FIRST).await;
    let jobs: Vec<(&str, JobPhase)> = history
        .subagents
        .iter()
        .map(|child| (child.subagent.description.as_str(), child.subagent.phase))
        .collect();
    assert_eq!(jobs, [("reader", JobPhase::Completed)]);
    backend.close().await;
}

// Over two seconds: the parent's and the child's shell jobs run on a real
// device, the child's until the Fork is taken.
#[tokio::test]
async fn a_fork_taken_while_a_child_runs_leaves_the_child_with_its_source() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _paired, root, _provider) = tree(&scripts).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    scripts.child(vec![shell("c1", WAIT), say("the child's result")]);
    scripts.root(
        FIRST,
        vec![
            shell(
                "t1",
                "demi agent spawn --description worker <<< 'Wait for the file'",
            ),
            say("the child is still working"),
            say("received"),
        ],
    );
    socket.chat("m1", "Start a worker").await;

    let text = match socket
        .live()
        .await
        .iter()
        .rev()
        .find(|block| matches!(block, Block::Text(_)))
    {
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
    assert_eq!(
        created.status,
        StatusCode::CREATED,
        "{}",
        String::from_utf8_lossy(&created.body)
    );
    // The Fork keeps the call that spawned the child, and no child.
    let before = transcript(&backend, &master, SECOND).await;
    assert!(
        before
            .blocks
            .iter()
            .any(|block| matches!(block, Block::ToolCall(_))),
        "{:?}",
        before.blocks
    );
    assert!(before.subagents.is_empty(), "{:?}", before.subagents);

    // The child finishes into its source alone.
    std::fs::write(format!("{root}/go"), "").unwrap();
    socket.until_idle().await;
    let source = transcript(&backend, &master, FIRST).await;
    assert_eq!(source.subagents.len(), 1);
    assert_eq!(transcript(&backend, &master, SECOND).await, before);

    // The Fork's tree has no child to list.
    let mut fork = Socket::connect(&backend, &master, SECOND).await;
    fork.open().await;
    scripts.root(
        SECOND,
        vec![shell("f1", "demi agent list"), say("an empty tree")],
    );
    fork.chat("m2", "Who works for you?").await;
    let asked = scripts.asked(|session| session == SECOND);
    let last = &asked[asked.len() - 1];
    let listed = &last[last
        .find("ToolResult { tool_use_id: \"f1\"")
        .expect("the list's result")..];
    assert!(
        listed.contains("(root session)") && !listed.contains("worker"),
        "{listed}"
    );
    backend.close().await;
}

/// The text of the last request of the sessions `keep` accepts.
fn last_asked(scripts: &Scripts, keep: impl Fn(&str) -> bool) -> String {
    scripts.asked(keep).pop().expect("a request was made")
}

// Several seconds: the root's three shell jobs run on a real device.
#[tokio::test]
async fn a_spawn_reads_the_users_settings_and_builds_the_childs_model_from_the_catalog() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _paired, _root, provider) = tree(&scripts).await;
    let explore = json!({
        "name": "explore", "description": "Finds code; it changes nothing.",
        "model": { "providerId": provider, "modelId": "n", "thinkingEffort": "high", "serviceTierId": null },
        "instructions": "You only report.", "canSpawn": false
    });
    let created = backend
        .post("/api/subagents/profiles", Some(&master), explore)
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    scripts.child(vec![say("found it")]);
    scripts.root(
        FIRST,
        vec![
            shell(
                "t1",
                "demi agent profiles && demi agent spawn --profile explore <<< 'Find the parser'",
            ),
            say("dispatched"),
            say("received"),
        ],
    );
    socket.chat("m1", "Find the parser").await;
    // The child's completion wakes the root, which answers it.
    socket
        .until(|frame| {
            matches!(
                frame,
                ServerFrame::Subagent {
                    event: SubagentEvent::Closed,
                    ..
                }
            )
        })
        .await;
    socket.until_idle().await;
    let child = last_asked(&scripts, |session| session != FIRST);
    assert!(child.contains("model=n "), "{child}");
    assert!(child.contains("effort: \"high\""), "{child}");
    assert!(child.contains("system=\"You only report."), "{child}");
    let listed = &scripts.asked(|session| session == FIRST)[1];
    assert!(
        listed.contains("explore  Finds code; it changes nothing."),
        "{listed}"
    );

    // Turned off in settings, the open tree's next spawn fails.
    let off = backend
        .put("/api/subagents", &master, json!({ "enabled": false }))
        .await;
    assert_eq!(off.status, StatusCode::NO_CONTENT);
    scripts.root(
        FIRST,
        vec![
            shell("t2", "demi agent spawn <<< 'Find it again'"),
            say("refused"),
        ],
    );
    socket.chat("m2", "Again").await;
    let refused = last_asked(&scripts, |session| session == FIRST);
    assert!(
        refused.contains("subagents are turned off in settings"),
        "{refused}"
    );
    backend.close().await;
}

// Several seconds: the child's shell job runs on a real device until the
// reset holds the conversation.
#[tokio::test]
async fn a_reset_holds_a_conversation_whose_subagent_infers_with_a_provider_on_the_cloud() {
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, _paired, _root, _provider) = tree(&scripts).await;
    // Its files and commands and its root's provider are on the device;
    // only the provider of its child's profile runs on the Cloud.
    let created = backend
        .post(
            "/api/providers",
            Some(&master),
            json!({
                "source": "custom", "providerType": "tree-process", "label": "On the Cloud",
                "apiKey": "k",
                "models": [{
                    "id": "c", "displayName": "C", "contextWindow": 100000, "outputLimit": null,
                    "thinkingEfforts": [], "acceptedExtensions": null, "fastTier": null
                }]
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let process = created.json::<ProviderAnswer>().provider.id;
    let profile = json!({
        "name": "cloud", "description": "Runs on the Cloud's provider.",
        "model": { "providerId": process, "modelId": "c", "thinkingEffort": null, "serviceTierId": null },
        "instructions": null, "canSpawn": true
    });
    let created = backend
        .post("/api/subagents/profiles", Some(&master), profile)
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    scripts.child(vec![shell("c1", WAIT), say("never said")]);
    scripts.root(
        FIRST,
        vec![
            shell("t1", "demi agent spawn --profile cloud <<< 'Wait for go'"),
            say("dispatched"),
        ],
    );
    socket.chat("m1", "Delegate").await;
    eventually("the child runs its command", || async {
        scripts.asked(|session| session != FIRST).len() == 1
    })
    .await;
    drop(socket);

    let (held, proceed) = (Arc::new(Notify::new()), Arc::new(Notify::new()));
    harness
        .manager
        .script(|script| script.hold_reset = Some((held.clone(), proceed.clone())));
    reset(&backend, &master, "6e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a19").await;
    held.notified().await;
    // The reset holds the conversation for its child: an open waits at its
    // file gate and goes on when the reset ends.
    let mut waiting = backend.file_gate(&master, FIRST).await.waiting();
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    {
        let opening = socket.open();
        tokio::pin!(opening);
        tokio::select! {
            _ = &mut opening => panic!("the open did not wait for the reset"),
            waits = waiting.wait_for(|count| *count > 0) => assert!(waits.is_ok()),
        }
        proceed.notify_one();
        opening.await;
    }
    backend.close().await;
}
