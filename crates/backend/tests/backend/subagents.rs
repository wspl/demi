//! Subagents through a conversation (`subagents.md`, `sessions-and-targets.md`
//! § Host operations, § Switch the primary target, `conversation-fork.md`):
//! the root and its child each learn the Host they work on before their
//! first request, once; a child works on the
//! conversation's Host in its parent's files, keeps its own identity even
//! for a command it runs through `demi host shell`, and runs on after
//! its spawn command has exited; a Fork taken while a child runs leaves the
//! child with its source; a spawn reads the user's subagent settings and
//! builds its child's model from the catalog; a live tree's subagent list
//! is the one its open sends; and a Cloud reset holds a
//! conversation whose child's profile infers with a provider on the Cloud.
//! Every node gets the project directory's skills and no others, and a spawn
//! takes the workflow the backend holds without asking a Host.
//! The model is a scripted family that answers each
//! node of a tree from a script of its own; the device is a real runner. No
//! test calls a real model.

use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use demi_backend_providers::llm::families::{
    FamilyArgs, FamilyCredential, FamilyError, FamilyRegistry, ProviderFamily,
};
use demi_conversation_socket_protocol::{
    ClientFrame, JobModel, JobPhase, ServerFrame, SubagentEvent, SubagentJob,
};
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
use demi_web_api_protocol::conversations::Subagents;
use demi_web_api_protocol::providers::{CredentialKind, ProviderAnswer};
use futures_util::future::{BoxFuture, LocalBoxFuture};
use futures_util::{StreamExt as _, stream};
use reqwest::StatusCode;
use serde_json::json;
use tokio::sync::Notify;

use crate::conversations::{FIRST, SECOND, Socket, choose, create, on_device, transcript};
use crate::support::{Harness, Session, TestBackend, eventually};
use demi_plugin_skills::testing::skill_md;

/// One scripted answer: the events of one request.
pub(crate) type Answer = Vec<ProviderEvent>;

/// The answers of each node: a root's by its session, which is its
/// conversation's id, and each child's in the order the children first ask.
#[derive(Default)]
pub(crate) struct Scripts {
    state: Mutex<ScriptState>,
    /// The next read of the provider's health, held: it says it arrived on
    /// the first, and goes on once the second is notified.
    health_hold: Mutex<Option<(Arc<Notify>, Arc<Notify>)>>,
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

    /// More answers for the one child that has asked, such as for its
    /// requests after a restart.
    pub(crate) fn child_again(&self, answers: Vec<Answer>) {
        let mut state = self.state.lock().unwrap();
        let mut asked = state.children.values_mut();
        let script = asked.next().expect("a child has asked");
        assert!(asked.next().is_none(), "one child has asked");
        script.extend(answers);
    }

    /// Holds the next read of the provider's health, as a catalog read
    /// makes it: answers what says the read arrived and what lets it go on.
    pub(crate) fn hold_next_health(&self) -> (Arc<Notify>, Arc<Notify>) {
        let hold = (Arc::new(Notify::new()), Arc::new(Notify::new()));
        *self.health_hold.lock().unwrap() = Some(hold.clone());
        hold
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
            "shell",
            json!({ "description": id, "script": script, "intervalMs": 60_000 }),
        ),
        event::response(1, 1),
    ]
}

/// A request the model never answers: it runs until it is cancelled, as by
/// a Stop or a shutdown.
pub(crate) fn hang() -> Answer {
    Vec::new()
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
        _: RuntimeEnv,
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
        let hold = self.scripts.health_hold.lock().unwrap().take();
        Box::pin(async move {
            if let Some((arrived, release)) = hold {
                arrived.notify_one();
                release.notified().await;
            }
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
        ProviderFailureFacts { retry_at: None, continue_with: None, signed_out: None }
    }

    fn runtime(&self, _: RuntimeEnv) -> Result<Box<dyn ProviderRuntime>, RuntimeError> {
        Ok(Box::new(TreeRuntime(self.scripts.clone())))
    }
}

struct TreeRuntime(Arc<Scripts>);

impl ProviderRuntime for TreeRuntime {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        let answer = self.0.answer(&request);
        let cancelled = request.cancel.cancelled_owned();
        if answer.is_empty() {
            return stream::pending().take_until(cancelled).boxed_local();
        }
        stream::iter(answer).take_until(cancelled).boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(Self(self.0.clone()))
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {})
    }

    /// Its wire carries images in a tool result, as the vendors' do, so a
    /// model that reads them sees what `demi file view` shows.
    fn request_limits(&self, _model: &demi_shared_types::Model) -> RequestLimits {
        RequestLimits {
            tool_results: demi_provider_common::ToolResultKinds::IMAGES,
            ..RequestLimits::default()
        }
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
    tree_among(scripts, harness, demi_backend::families::builtin()).await
}

/// [`tree_on`], with the tree's families beside the families of `base`.
pub(crate) async fn tree_among(
    scripts: &Arc<Scripts>,
    harness: Harness,
    base: FamilyRegistry,
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
    let families = base
        .with("tree", tree(false))
        .with("tree-process", tree(true));
    let harness = harness.with_families(families);
    let (backend, master) = harness.start_set_up().await;
    let created = backend
        .post(
            "/api/providers",
            Some(&master),
            json!({
                "source": "family", "providerType": "tree", "label": "Tree", "apiKey": "k",
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

    // The root learned its Host before its first request, and the child
    // before its own: its system as the runner reported it, and the
    // directory its shells start in.
    let host = "[Execution context 0]\\nPrimary host: laptop, ";
    let started = format!(
        " ({}). Shells start in {root}. {} {}\\nAttached hosts: none.",
        std::env::consts::ARCH,
        crate::support::SCRATCH,
        crate::support::device_utilities()
    );
    let roots = scripts.asked(|session| session == FIRST);
    let children = scripts.asked(|session| session != FIRST);
    for first in [&roots[0], &children[0]] {
        let block = first.find(&host).map(|at| &first[at + host.len()..]);
        let system = block.and_then(|block| block.find(&started).map(|end| &block[..end]));
        assert!(
            system.is_some_and(|system| !system.is_empty() && !system.contains("\\n")),
            "{first}"
        );
    }
    // The root saw this revision: its later requests carry the one block
    // its history holds.
    let latest = roots.last().unwrap();
    assert_eq!(latest.matches("[Execution context ").count(), 1, "{latest}");
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
        .map(|child| (child.child.description.as_str(), child.child.phase))
        .collect();
    assert_eq!(jobs, [("reader", JobPhase::Completed)]);
    backend.close().await;
}

// Several seconds: a parent and its child run eight shell jobs on a real
// device, across a resume of the child.
#[tokio::test]
async fn a_child_works_in_its_own_directory_which_it_changes_and_keeps_and_the_root_keeps_the_conversations() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _paired, root, _provider) = tree(&scripts).await;
    for directory in ["app", "lib"] {
        std::fs::create_dir(format!("{root}/{directory}")).unwrap();
    }
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // The child starts where `--cwd` names, against its parent's directory,
    // once the test lets it go; it moves to `../lib`, against its own, from
    // its next call on; a directory the device does not have is refused and
    // changes nothing.
    scripts.child(vec![
        shell("c1", &format!("{WAIT}; echo \"first=$(pwd)\" && demi agent cwd ../lib")),
        shell("c2", "echo \"second=$(pwd)\"; demi agent cwd missing; echo \"refused=$?\""),
        shell("c3", "echo \"third=$(pwd)\""),
        say("moved"),
        // Its next round, after a resume restored it from the store.
        shell("c4", &format!("until [ -f go2 ]; do sleep 0.05; done; echo \"fourth=$(pwd)\"")),
        say("still there"),
    ]);
    scripts.root(
        FIRST,
        vec![
            shell("t1", "demi agent cwd ../elsewhere; echo \"root=$?\"; pwd"),
            shell(
                "t2",
                "demi agent spawn --cwd app --description mover <<< 'Move to the library'",
            ),
            say("dispatched"),
            say("received"),
        ],
    );
    let mut frames = socket.chat("m1", "Delegate the move").await;
    std::fs::write(format!("{root}/app/go"), "").unwrap();
    frames.extend(socket.until_idle().await);

    scripts.root(
        FIRST,
        vec![
            shell("t3", "demi agent resume 1 <<< 'Look again'"),
            say("resumed"),
            say("received again"),
        ],
    );
    frames.extend(socket.chat("m2", "Once more").await);
    std::fs::write(format!("{root}/lib/go2"), "").unwrap();
    frames.extend(socket.until_idle().await);

    // The root's directory is the conversation's: its change is refused
    // with the reason, and its shells still start there.
    let refused = last_asked(&scripts, |session| session == FIRST);
    assert!(
        refused.contains("The main agent works in the conversation's directory; move the conversation with `demi host move` to work elsewhere.")
            && refused.contains(&format!("root=1\\n{root}")),
        "{refused}"
    );
    let children = scripts.asked(|session| session != FIRST);
    let told = |cwd: &str| format!("Your working directory is {cwd}: ");
    assert!(children[0].contains(&told(&format!("{root}/app"))), "{}", children[0]);
    assert!(children[1].contains(&told(&format!("{root}/lib"))), "{}", children[1]);
    let last = children.last().unwrap();
    for ran in [
        format!("first={root}/app"),
        format!("second={root}/lib"),
        format!("{root}/lib/missing: no such directory on the primary Host"),
        "refused=1".to_owned(),
        format!("third={root}/lib"),
        format!("fourth={root}/lib"),
    ] {
        assert!(last.contains(&ran), "{ran} in {last}");
    }
    // The page learns where the child works: as it starts, as it moves, and
    // in the conversation's list of subagents.
    let moves: Vec<(SubagentEvent, String)> = frames
        .iter()
        .filter_map(|frame| match frame {
            ServerFrame::Subagent { event, job } => Some((*event, job.child.cwd.clone())),
            _ => None,
        })
        .collect();
    assert!(moves.contains(&(SubagentEvent::Started, format!("{root}/app"))), "{moves:?}");
    assert!(moves.contains(&(SubagentEvent::Changed, format!("{root}/lib"))), "{moves:?}");
    let history = transcript(&backend, &master, FIRST).await;
    let listed: Vec<&str> = history.subagents.iter().map(|child| child.child.cwd.as_str()).collect();
    assert_eq!(listed, [format!("{root}/lib")]);
    backend.close().await;
}

// Several seconds: a parent and its child run six shell jobs on a real
// device, across a switch of the conversation's directory.
#[tokio::test]
async fn a_move_of_the_conversation_puts_its_children_in_its_new_directory_and_list_and_show_name_them() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, paired, root, _provider) = tree(&scripts).await;
    std::fs::create_dir(format!("{root}/app")).unwrap();
    let moved = format!("{root}/../moved");
    std::fs::create_dir(&moved).unwrap();
    let moved = std::fs::canonicalize(&moved).unwrap().to_str().unwrap().to_owned();
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    scripts.child(vec![
        shell("c1", "echo \"before=$(pwd)\""),
        say("here"),
        // Its next round, after the conversation moved.
        shell("c2", "echo \"after=$(pwd)\""),
        say("there"),
    ]);
    scripts.root(
        FIRST,
        vec![
            shell("t1", "demi agent spawn --cwd app --description mover <<< 'Say where you are'"),
            say("dispatched"),
            say("received"),
            // `list` names the archived child's directory against the
            // conversation's, `show` whole.
            shell("t2", "demi agent list; demi agent show 1"),
            say("listed"),
        ],
    );
    socket.chat("m1", "Delegate").await;
    socket.until_idle().await;
    socket.chat("m2", "List them").await;
    let listed = last_asked(&scripts, |session| session == FIRST);
    assert!(listed.contains("\\\"mover\\\"  app"), "{listed}");
    assert!(listed.contains(&format!("cwd: {root}/app")), "{listed}");

    // The user moves the conversation to another directory of the device.
    let target = json!({ "target": { "kind": "device", "deviceId": paired.id(), "path": moved } });
    let switched = backend.patch(&format!("/api/conversations/{FIRST}"), &master, target).await;
    assert_eq!(switched.status, StatusCode::OK, "{}", String::from_utf8_lossy(&switched.body));

    scripts.root(
        FIRST,
        vec![
            shell("t3", "demi agent resume 1 <<< 'Where are you now?'"),
            say("resumed"),
            say("received again"),
        ],
    );
    socket.chat("m3", "Ask again").await;
    socket.until_idle().await;

    let children = scripts.asked(|session| session != FIRST);
    let last = children.last().unwrap();
    assert!(last.contains(&format!("before={root}/app")), "{last}");
    assert!(last.contains(&format!("after={moved}")), "{last}");
    // The resumed child was told where it works now.
    assert!(last.contains(&format!("Your working directory is {moved}: ")), "{last}");
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

#[tokio::test]
async fn a_spawn_reads_the_users_settings_and_builds_the_childs_model_from_the_catalog() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _paired, root, provider) = tree(&scripts).await;
    let explore = json!({
        "name": "explore", "description": "Finds code; it changes nothing.",
        "model": { "providerId": provider, "modelId": "n", "thinkingEffort": "high", "serviceTierId": null },
        "contextLimit": null, "instructions": "You only report.", "canSpawn": false
    });
    let created = backend
        .post("/api/subagents/profiles", Some(&master), explore)
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // The child waits until the root's turn has ended: a completion that
    // came while that turn still ran would join it at its next boundary,
    // and no turn of its own would follow.
    scripts.child(vec![shell("c1", WAIT), say("found it")]);
    scripts.root(
        FIRST,
        vec![
            shell(
                "t1",
                "demi agent profiles list && demi agent spawn --profile explore <<< 'Find the parser'",
            ),
            say("dispatched"),
            say("received"),
        ],
    );
    socket.chat("m1", "Find the parser").await;
    std::fs::write(format!("{root}/go"), "").unwrap();
    // The child's completion wakes the idle root, which answers it.
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
    // The profile adds to Demi's prompt and replaces none of it.
    assert!(child.contains("system=\"You are Demi, "), "{child}");
    assert!(
        child.contains("answer with this.\\n\\nInstructions for this subagent:\\n\\nYou only report.\" items="),
        "{child}"
    );
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

/// The windows the `context_usage` frames among `frames` report, the root's
/// and the other nodes' apart.
fn usage_windows(frames: &[ServerFrame]) -> (Vec<u64>, Vec<u64>) {
    let (mut root, mut others) = (Vec::new(), Vec::new());
    for frame in frames {
        if let ServerFrame::ContextUsage {
            node,
            usage: Some(usage),
        } = frame
            && let Some(window) = usage.window
        {
            if node.as_str() == FIRST { &mut root } else { &mut others }.push(window);
        }
    }
    (root, others)
}

// A backend, a real device's shell jobs and a child that compacts: about a
// second.
#[tokio::test]
async fn a_profiles_context_replaces_the_users_limit_for_its_children_alone() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _paired, root, _) = tree(&scripts).await;
    let wide = backend
        .post(
            "/api/providers",
            Some(&master),
            json!({
                "source": "family", "providerType": "tree", "label": "Wide", "apiKey": "k",
                "models": [{
                    "id": "wide", "displayName": "Wide", "contextWindow": 1000000, "outputLimit": null,
                    "thinkingEfforts": [], "acceptedExtensions": null, "fastTier": null
                }]
            }),
        )
        .await;
    assert_eq!(wide.status, StatusCode::CREATED);
    let wide = wide.json::<ProviderAnswer>().provider.id.as_str().to_owned();
    choose(&backend, &master, FIRST, &wide, "wide").await;
    let model = json!({ "providerId": wide, "modelId": "wide", "thinkingEffort": null, "serviceTierId": null });
    let profile = |name: &str, model: serde_json::Value, context: serde_json::Value| {
        json!({
            "name": name, "description": "Looks.", "model": model, "contextLimit": context,
            "instructions": null, "canSpawn": false
        })
    };

    // A Context needs a model of the profile's own, one its window offers.
    for (body, refusal) in [
        (
            profile("parent", serde_json::Value::Null, json!(500000)),
            "context: the profile inherits the parent's model",
        ),
        (
            profile("odd", model.clone(), json!(400000)),
            "context: model \\\"wide\\\" does not offer 400K; it offers full (1M), 500K, 300K, 200K, or default for the user's limit",
        ),
    ] {
        let refused = backend.post("/api/subagents/profiles", Some(&master), body).await;
        let text = String::from_utf8_lossy(&refused.body);
        assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{text}");
        assert!(text.contains("invalid_body") && text.contains(refusal), "{text}");
    }
    let created = backend
        .post("/api/subagents/profiles", Some(&master), profile("half", model, json!(500000)))
        .await;
    assert_eq!(created.status, StatusCode::CREATED);

    // The user limits the model to 300K: the root uses it, the profile's
    // children their 500K.
    let limit = json!({ "contextLimit": { "providerId": wide, "modelId": "wide", "tokens": 300000 } });
    let limited = backend.patch("/api/settings/preferences", &master, limit).await;
    assert_eq!(limited.status, StatusCode::OK);

    // The child's shell call reports 480,000 tokens: over the 467,000 at
    // which a window of 500K compacts, so its next request follows a pass.
    // The root's answer reports 250,000: under the 267,000 of the user's
    // 300K.
    let mut looked = shell("c2", "true");
    looked[1] = event::response(480_000, 5);
    scripts.child(vec![
        shell("c1", WAIT),
        looked,
        say("The parent asked to look."),
        say("found it"),
        shell("c3", "until [ -f again ]; do sleep 0.05; done"),
        say("looked again"),
    ]);
    scripts.root(
        FIRST,
        vec![
            shell("t1", "demi agent spawn --profile half <<< 'Look'"),
            vec![event::text("dispatched"), event::response(250_000, 5)],
            say("received"),
        ],
    );
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    let closed = |frame: &ServerFrame| {
        matches!(
            frame,
            ServerFrame::Subagent {
                event: SubagentEvent::Closed,
                ..
            }
        )
    };
    let mut frames = socket.chat("m1", "Look").await;
    std::fs::write(format!("{root}/go"), "").unwrap();
    frames.extend(socket.until(closed).await);
    frames.extend(socket.until_idle().await);

    let compacting = |frames: &[ServerFrame], subagent: bool| {
        frames.iter().any(|frame| {
            matches!(
                frame,
                ServerFrame::Phase { subagent_id, phase: demi_shared_types::SessionPhase::Compacting }
                    if subagent_id.is_some() == subagent
            )
        })
    };
    assert!(compacting(&frames, true), "the child compacts at the threshold of 500K");
    assert!(!compacting(&frames, false), "the root's answer is under the user's 300K");
    assert_eq!(scripts.asked(|session| session != FIRST).len(), 4);
    assert_eq!(scripts.asked(|session| session == FIRST).len(), 3);
    let (roots, children) = usage_windows(&frames);
    assert!(!roots.is_empty() && roots.iter().all(|window| *window == 300_000), "{roots:?}");
    assert!(!children.is_empty() && children.iter().all(|window| *window == 500_000), "{children:?}");

    // The profile goes to 200K; the child resumed keeps the 500K it was
    // spawned with, as it keeps its instructions.
    let id = created.json::<demi_web_api_protocol::subagents::ProfileAnswer>().profile.id;
    let edited = backend
        .patch(&format!("/api/subagents/profiles/{id}"), &master, json!({ "contextLimit": 200000 }))
        .await;
    assert_eq!(edited.status, StatusCode::OK, "{}", String::from_utf8_lossy(&edited.body));
    scripts.root(
        FIRST,
        vec![
            shell("t2", "demi agent resume 1 <<< 'Look again'"),
            say("resumed"),
            say("received again"),
        ],
    );
    let mut frames = socket.chat("m2", "Again").await;
    std::fs::write(format!("{root}/again"), "").unwrap();
    frames.extend(socket.until(closed).await);
    frames.extend(socket.until_idle().await);
    let (_, children) = usage_windows(&frames);
    assert!(!children.is_empty() && children.iter().all(|window| *window == 500_000), "{children:?}");
    assert_eq!(scripts.asked(|session| session != FIRST).len(), 6);
    backend.close().await;
}

/// The Context a profile keeps through its model's changes (`subagents.md`
/// § Profiles).
// A backend and a few requests, no conversation: well under a second.
#[tokio::test]
async fn a_profiles_context_stays_while_its_model_offers_it_and_goes_with_the_parents_model() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _paired, _root, small) = tree(&scripts).await;
    let wide = backend
        .post(
            "/api/providers",
            Some(&master),
            json!({
                "source": "family", "providerType": "tree", "label": "Wide", "apiKey": "k",
                "models": [
                    {
                        "id": "wide", "displayName": "Wide", "contextWindow": 1000000, "outputLimit": null,
                        "thinkingEfforts": [], "acceptedExtensions": null, "fastTier": null
                    },
                    {
                        "id": "medium", "displayName": "Medium", "contextWindow": 800000, "outputLimit": null,
                        "thinkingEfforts": [], "acceptedExtensions": null, "fastTier": null
                    }
                ]
            }),
        )
        .await;
    assert_eq!(wide.status, StatusCode::CREATED);
    let wide = wide.json::<ProviderAnswer>().provider.id.as_str().to_owned();
    let model = |provider: &str, id: &str| {
        json!({ "providerId": provider, "modelId": id, "thinkingEffort": null, "serviceTierId": null })
    };
    let created = backend
        .post(
            "/api/subagents/profiles",
            Some(&master),
            json!({
                "name": "scout", "description": "Looks.", "model": model(&wide, "wide"),
                "contextLimit": 300000, "instructions": null, "canSpawn": false
            }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let id = created.json::<demi_web_api_protocol::subagents::ProfileAnswer>().profile.id;
    let path = format!("/api/subagents/profiles/{id}");
    let patched = |answer: crate::support::Answer| {
        assert_eq!(answer.status, StatusCode::OK, "{}", String::from_utf8_lossy(&answer.body));
        let profile = answer.json::<demi_web_api_protocol::subagents::ProfileAnswer>().profile;
        (profile.model.map(|model| model.model_id), profile.context_limit)
    };

    // Medium's 800K offers 300K, which stays; the small model offers no
    // choice, and the parent's model takes none: both go back to default.
    for (body, after) in [
        (json!({ "model": model(&wide, "medium") }), (Some("medium"), Some(300_000))),
        (json!({ "model": model(&small, "m") }), (Some("m"), None)),
        (json!({ "model": model(&wide, "wide"), "contextLimit": 500000 }), (Some("wide"), Some(500_000))),
        (json!({ "model": null }), (None, None)),
    ] {
        let (model, context) = patched(backend.patch(&path, &master, body.clone()).await);
        assert_eq!((model.as_deref(), context), after, "{body}");
    }

    // A Context for the parent's model is refused.
    let refused = backend.patch(&path, &master, json!({ "contextLimit": 500000 })).await;
    let text = String::from_utf8_lossy(&refused.body);
    assert_eq!(refused.status, StatusCode::BAD_REQUEST, "{text}");
    assert!(text.contains("context: the profile inherits the parent's model"), "{text}");
}

/// The `started` jobs among `frames`.
fn started(frames: &[ServerFrame]) -> Vec<SubagentJob> {
    frames
        .iter()
        .filter_map(|frame| match frame {
            ServerFrame::Subagent {
                event: SubagentEvent::Started,
                job,
            } => Some(job.clone()),
            _ => None,
        })
        .collect()
}

// About a second: four rounds of children each run a shell job on a real
// device.
#[tokio::test]
async fn a_childs_job_says_what_its_round_runs_on_in_the_menus_names() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _paired, root, provider) = tree(&scripts).await;
    // A second entry whose `n` has a Fast tier, and a profile on it with
    // thinking off and the Fast tier.
    let quick = backend
        .post(
            "/api/providers",
            Some(&master),
            json!({
                "source": "family", "providerType": "tree", "label": "Quick", "apiKey": "k",
                "models": [{
                    "id": "n", "displayName": "N", "contextWindow": 100000, "outputLimit": null,
                    "thinkingEfforts": ["low", "high"], "acceptedExtensions": null,
                    "fastTier": "priority"
                }]
            }),
        )
        .await;
    assert_eq!(quick.status, StatusCode::CREATED);
    let quick = quick.json::<ProviderAnswer>().provider.id;
    let profile = json!({
        "name": "quick", "description": "Answers fast.",
        "model": { "providerId": quick, "modelId": "n", "thinkingEffort": "disabled", "serviceTierId": "priority" },
        "contextLimit": null, "instructions": null, "canSpawn": false
    });
    let created = backend
        .post("/api/subagents/profiles", Some(&master), profile)
        .await;
    assert_eq!(created.status, StatusCode::CREATED);
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    // Each round's child waits until the root's turn has ended, so that its
    // completion wakes the idle root, which answers it.
    let go = format!("{root}/go");
    let mut round = async |id: &str, command: &str| {
        let _ = std::fs::remove_file(&go);
        scripts.root(
            FIRST,
            vec![shell(id, command), say("dispatched"), say("received")],
        );
        let mut frames = socket.chat(id, "Go").await;
        std::fs::write(&go, "").unwrap();
        frames.extend(
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
                .await,
        );
        frames.extend(socket.until_idle().await);
        started(&frames)
    };
    let inherited = JobModel {
        provider_id: provider,
        provider: "Tree".to_owned(),
        model: "M".to_owned(),
        effort: None,
        fast: false,
    };
    let fast = JobModel {
        provider_id: quick.to_string(),
        provider: "Quick".to_owned(),
        model: "N".to_owned(),
        effort: Some("off".to_owned()),
        fast: true,
    };
    let runs_on = |jobs: &[SubagentJob]| -> Vec<(u64, Option<String>, JobModel, u64, Option<u64>)> {
        jobs.iter()
            .map(|job| {
                let job = job.clone();
                (job.child.number, job.child.profile, job.child.model, job.child.round, job.child.forked_from)
            })
            .collect()
    };

    // A child without a profile runs on its parent's model.
    scripts.child(vec![shell("c1", WAIT), say("first"), shell("c3", WAIT), say("again")]);
    let jobs = round("t1", "demi agent spawn <<< 'Look'").await;
    assert_eq!(runs_on(&jobs), [(1, None, inherited.clone(), 1, None)]);

    // A profile's child runs on the profile's model.
    scripts.child(vec![shell("c2", WAIT), say("quick")]);
    let jobs = round("t2", "demi agent spawn --profile quick <<< 'Look fast'").await;
    assert_eq!(
        runs_on(&jobs),
        [(2, Some("quick".to_owned()), fast.clone(), 1, None)]
    );

    // A resumed child starts its second round on the same model.
    let jobs = round("t3", "demi agent resume 1 <<< 'Look again'").await;
    assert_eq!(runs_on(&jobs), [(1, None, inherited.clone(), 2, None)]);

    // A fork of the profile's child without a profile runs on its
    // parent's model, which it switches to from its source's, and says
    // which agent it was forked from.
    scripts.child(vec![shell("c4", WAIT), say("forked")]);
    let jobs = round("t4", "demi agent fork 2 <<< 'Look once more'").await;
    assert_eq!(runs_on(&jobs), [(3, None, inherited.clone(), 1, Some(2))]);

    // The conversation's subagents route says the same of the closed ones.
    let listed = transcript(&backend, &master, FIRST).await.subagents;
    assert_eq!(
        runs_on(&listed),
        [
            (1, None, inherited.clone(), 2, None),
            (2, Some("quick".to_owned()), fast, 1, None),
            (3, None, inherited, 1, Some(2))
        ]
    );
    backend.close().await;
}

// Half a second: a parent and its child each run a shell job on a real
// device, and the tree is restored once.
#[tokio::test]
async fn a_live_trees_subagent_list_is_the_one_its_open_sends() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _paired, root, provider) = tree(&scripts).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    // The child waits until its parent's turn has ended, so that its
    // completion wakes the idle parent.
    scripts.root(
        FIRST,
        vec![shell("t1", "demi agent spawn <<< 'Look'"), say("dispatched"), say("received")],
    );
    scripts.child(vec![shell("c1", WAIT), say("found")]);
    socket.chat("t1", "Go").await;
    std::fs::write(format!("{root}/go"), "").unwrap();
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
    // The entry's new name reaches the store's reading of the child, not
    // the live tree's, whose summary names what its round ran on.
    let renamed = backend
        .patch(
            &format!("/api/providers/{provider}"),
            &master,
            json!({ "label": "Renamed" }),
        )
        .await;
    assert_eq!(renamed.status, StatusCode::OK);

    let listed = backend
        .get(&format!("/api/conversations/{FIRST}/subagents"), Some(&master))
        .await;
    assert_eq!(listed.status, StatusCode::OK);
    let listed: Vec<_> = listed
        .json::<Subagents>()
        .subagents
        .into_iter()
        .map(|job| job.child)
        .collect();
    let mut page = Socket::connect(&backend, &master, FIRST).await;
    page.open().await;
    let opened = page
        .until(|frame| matches!(frame, ServerFrame::Subagents { .. }))
        .await;
    let Some(ServerFrame::Subagents { subagents }) = opened.last() else {
        unreachable!("the handshake ends at its subagents frame")
    };
    let sent: Vec<_> = subagents.iter().map(|summary| summary.child.clone()).collect();
    assert_eq!(listed, sent);
    assert_eq!(
        sent.iter().map(|child| (child.number, child.phase, child.model.provider.as_str())).collect::<Vec<_>>(),
        [(1, JobPhase::Completed, "Tree")]
    );

    // A list read from the store while no tree is live, whose tree opens
    // before it answers, after the entry was renamed again: the answer is
    // the tree's, the handshake's.
    page.send(&ClientFrame::Close {}).await;
    page.until(|frame| *frame == ServerFrame::Closed).await;
    let (arrived, release) = scripts.hold_next_health();
    let path = format!("/api/conversations/{FIRST}/subagents");
    let (listed, sent) = tokio::join!(
        backend.get(&path, Some(&master)),
        async {
            arrived.notified().await;
            let renamed = backend
                .patch(&format!("/api/providers/{provider}"), &master, json!({ "label": "Again" }))
                .await;
            assert_eq!(renamed.status, StatusCode::OK);
            let mut page = Socket::connect(&backend, &master, FIRST).await;
            page.open().await;
            let opened = page
                .until(|frame| matches!(frame, ServerFrame::Subagents { .. }))
                .await;
            release.notify_one();
            let Some(ServerFrame::Subagents { subagents }) = opened.last() else {
                unreachable!("the handshake ends at its subagents frame")
            };
            subagents.iter().map(|summary| summary.child.clone()).collect::<Vec<_>>()
        },
    );
    assert_eq!(listed.status, StatusCode::OK);
    let listed: Vec<_> = listed.json::<Subagents>().subagents.into_iter().map(|job| job.child).collect();
    assert_eq!(listed, sent);
    assert_eq!(sent[0].model.provider, "Again");
    backend.close().await;
}

/// Writes the skill `name` into `.agents/skills` of `directory`.
fn project_skill(directory: &std::path::Path, name: &str) {
    let skill = directory.join(".agents/skills").join(name);
    std::fs::create_dir_all(&skill).unwrap();
    std::fs::write(skill.join("SKILL.md"), skill_md(&format!("name: {name}\ndescription: The skill {name}."))).unwrap();
}

// About a second: a real runner on a paired device answers the search, and
// the root's one shell job spawns the child.
//
// Planted defect it catches: a search from each node's own directory up to
// its repository's root, which finds `nested` and `outside` and not the
// project's own `release`.
#[tokio::test]
async fn a_child_elsewhere_gets_the_project_directorys_skills_and_none_from_another_directory() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, paired, root, _provider) = tree(&scripts).await;
    let home = paired.runner.home_dir().to_owned();
    let project = std::path::Path::new(&root);
    // The project is inside a repository whose root is the device's home.
    std::fs::create_dir_all(home.join(".git")).unwrap();
    project_skill(project, "release");
    project_skill(&project.join("app"), "nested");
    project_skill(&home, "outside");
    crate::work::into_project(&backend, &master, FIRST, &paired, project).await;
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;

    scripts.child(vec![say("here")]);
    scripts.root(
        FIRST,
        vec![
            shell("t1", "demi agent spawn --cwd app --description mover <<< 'Look around'"),
            say("dispatched"),
            say("received"),
        ],
    );
    socket.chat("m1", "Delegate").await;
    eventually("the child asked", || {
        let asked = !scripts.asked(|session| session != FIRST).is_empty();
        async move { asked }
    })
    .await;
    let child = scripts.asked(|session| session != FIRST).remove(0);
    let parent = scripts.asked(|session| session == FIRST).remove(0);
    for asked in [&child, &parent] {
        assert!(asked.contains("<name>release</name>"), "{asked}");
        assert!(!asked.contains("<name>nested</name>"), "{asked}");
        assert!(!asked.contains("<name>outside</name>"), "{asked}");
    }
    backend.close().await;
}

// A few seconds: two real runners; a job on the attached one stops the
// primary Host's runner and then spawns, through its own.
//
// Planted defect it catches: a spawn that reads the workflow on the Host,
// which waits for a Host that never answers.
#[tokio::test]
async fn a_spawn_takes_the_held_workflow_while_the_host_answers_nothing() {
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, laptop, root, _provider) = tree(&scripts).await;
    let alpha = backend.pair(&master, "alpha").await;
    let workflows = std::path::Path::new(&root).join(".demi/workflows");
    std::fs::create_dir_all(&workflows).unwrap();
    std::fs::write(
        workflows.join("team.yaml"),
        "name: Team\nprofiles:\n  - id: helper\n    description: Helps.\n",
    )
    .unwrap();
    // Working on alpha, then on the laptop again, leaves alpha attached.
    for (device, path) in [(&alpha, alpha.runner.home().to_owned()), (&laptop, root.clone())] {
        let target = json!({ "target": { "kind": "device", "deviceId": device.id(), "path": path } });
        let moved = backend.patch(&format!("/api/conversations/{FIRST}"), &master, target).await;
        assert_eq!(moved.status, StatusCode::OK, "{}", String::from_utf8_lossy(&moved.body));
    }
    let chosen = json!({ "workflow": { "type": "workflow", "id": "team" } });
    let chose = backend.patch(&format!("/api/conversations/{FIRST}"), &master, chosen).await;
    assert_eq!(chose.status, StatusCode::OK, "{}", String::from_utf8_lossy(&chose.body));
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    // The root's first request takes the workflow's instructions, which
    // holds the directory's workflows from then on.
    scripts.root(FIRST, vec![say("ready")]);
    socket.chat("m1", "Get ready").await;

    scripts.child(vec![say("helped")]);
    let script = format!(
        "demi host shell --host alpha 'kill -STOP {} && demi agent spawn --profile helper --description helper <<< Help'",
        laptop.runner.pid()
    );
    scripts.root(FIRST, vec![shell("t1", &script), say("dispatched"), say("received")]);
    // The turn's job runs on the stopped Host: its end waits for it.
    socket.send(&crate::conversations::send("m2", "Delegate")).await;
    eventually("the child asked", || {
        let asked = !scripts.asked(|session| session != FIRST).is_empty();
        async move { asked }
    })
    .await;
    laptop.runner.resume();
    socket.until_idle().await;
    backend.close().await;
}
