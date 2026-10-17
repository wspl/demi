//! The compaction fixture's harness (`README.md` beside it), the real-model
//! check of `compaction.md` § Acceptance, level 3: it opens the committed
//! large-context session as a conversation of an agent server, talks to the
//! real DeepSeek through the OpenAI-compatible provider, compacts it several
//! times, and after each pass checks that the facts, the constraint and the
//! open task planted at its start survive, that the model continues without
//! restating the summary, and that the summary request read the vendor's
//! cache. Each check's question goes to a throwaway copy of the
//! conversation, so no check's answer reaches the history a later pass
//! summarizes.
//!
//! ```sh
//! target/debug/examples/compaction-fixture recall --passes 3
//! target/debug/examples/compaction-fixture switch --passes 2
//! ```
//!
//! It calls a real model, so no test runs it; every build of the one
//! selection compiles it, as it compiles every example. It observes
//! compaction only through the transcript (a new `compaction_boundary`), the
//! session's context usage and what the provider reports, so it holds no
//! copy of the compaction rules.

use std::{
    cell::RefCell, collections::HashSet, io::Read, process::ExitCode, rc::Rc, sync::Arc,
    time::Duration,
};

use demi_agent_server::{
    AgentServer, ProviderResolver, ResolveError, ServerConfig, ServerDeps,
    testing::{TestClient, client_text},
};
use demi_agent_store::{
    AgentTreeStore, Checkpoint, CheckpointState, CheckpointUpdate, NodeRecord,
    testing::MemoryTreeStore,
};
use demi_agent_tools::{
    HostResolver, ProfileModel, SubagentSettings, Toolset, Unavailable,
    testing::{NoHost, NoShells},
};
use demi_agent_transcript::RandomIds;
use demi_conversation_socket_protocol::{ClientFrame, ServerFrame};
use demi_host_interface::CommandSet;
use demi_provider_common::{
    InferenceItem, InferenceRequest, Provider, ProviderEvent, ProviderRun, ProviderRuntime,
    RequestLimits, RuntimeEnv, Secret, VendorPolicy,
};
use demi_provider_openai_api::{OpenAiConfig, OpenAiProvider};
use demi_shared_types::{
    Block, Clock, ContextUsage, Model, ModelSelection, NodeId, SessionPhase, SystemClock,
    TokenUsage, ToolResultContentBlock, TurnId, UserContentBlock, WireApi,
};
use futures_util::{StreamExt as _, future::LocalBoxFuture};
use serde::Deserialize;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/compaction/large-context-fixture.json.gz"
);
const SYSTEM_PROMPT: &str = "You are a careful coding assistant.";

/// The sentence the fixture's first message ends with; what the harness
/// plants goes before it, so the recorded answer still follows.
const FIRST_MESSAGE_END: &str = "只回复「已记住」。";
/// The constraint and the open task planted at the start, beside the three
/// secrets the first message already holds.
const PLANTED: &str = "另外两件事也要一直记住:\
    一、约束:永远不要修改 `migrations/` 目录下的任何文件;需要改的时候,先问我。\
    二、未完成的任务:TASK-17,给 shell 包的超时修复写一条 CHANGELOG 记录。它还没做完,读完这些文件后我们接着做。";
const SECRETS: [(&str, &str); 3] = [("ZEBRA", "7"), ("QUARTZ", "9"), ("NIMBUS", "3")];
const OPEN_TASK: &str = "TASK-17";

/// What the harness asks after each pass, short so that the answers parse
/// simply.
const NEXT_PROMPT: &str = "接下来做什么?用一句话回答;如果有未完成的任务,说出它的编号。";
const CONSTRAINT_PROMPT: &str = "请在 `migrations/0042_users.sql` 里给 users.email 加一个索引。\
    先别动手,只用一行回答:如果你现在就会改,以 EDIT 开头;如果我定过的规则要求先问我,以 ASK 开头,后面写出那条规则。";
const RECALL_PROMPT: &str =
    "只回答暗号值,用「ALPHA=…, BETA=…, GAMMA=…」格式:我最早让你记住的三个暗号分别是什么?";
const FILLER_PROMPT: &str = "下面是一段参考资料,先不用分析,只回复 ok。";
/// The turn a model switch lands with: a switch takes effect, and compacts,
/// at the conversation's next request, which must not be a check's.
const LANDING_PROMPT: &str = "只回复 ok。";

/// How much reference text one filler turn carries.
const FILLER_CHARS: usize = 48_000;
/// The most filler turns one pass may take to fill the context.
const MAX_FILLER_TURNS: usize = 40;

/// How long one action of the real model may take: a turn that compacts
/// first sends a summary request of several hundred thousand tokens.
const TURN_TIMEOUT: Duration = Duration::from_secs(1_800);

/// The committed fixture: a session that compacted several times, in the
/// agent's block format.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Fixture {
    built_tokens: u64,
    generations: u32,
    blocks: Vec<Block>,
}

/// What a run does: its mode, its passes and the two windows.
struct Run {
    mode: Mode,
    passes: usize,
    small: u32,
    large: u32,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Recall,
    Switch,
}

fn main() -> ExitCode {
    let run = match parse_args() {
        Ok(run) => run,
        Err(error) => {
            eprintln!("compaction fixture: {error}");
            eprintln!("usage: compaction-fixture <recall|switch> [--passes N]");
            return ExitCode::FAILURE;
        }
    };
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build_local(tokio::runtime::LocalOptions::default())
        .expect("a local runtime starts");
    let passed = runtime.block_on(async {
        let fixture = load_fixture()?;
        match run.mode {
            Mode::Recall => recall(fixture, &run).await,
            Mode::Switch => switch(fixture, &run).await,
        }
    });
    match passed {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("compaction fixture: {error}");
            ExitCode::FAILURE
        }
    }
}

fn parse_args() -> Result<Run, String> {
    let mut args = std::env::args().skip(1);
    let mut mode = None;
    let mut passes = 3;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "recall" => mode = Some(Mode::Recall),
            "switch" => mode = Some(Mode::Switch),
            "--passes" => {
                let value = args.next().ok_or("--passes needs a number")?;
                passes = value
                    .parse()
                    .map_err(|error| format!("--passes {value}: {error}"))?;
            }
            other => return Err(format!("unknown argument {other:?}")),
        }
    }
    if passes == 0 {
        return Err("--passes must be at least 1".into());
    }
    Ok(Run {
        mode: mode.unwrap_or(Mode::Recall),
        passes,
        small: window("COMPACTION_FIXTURE_SMALL_WINDOW", 128_000)?,
        large: window("COMPACTION_FIXTURE_LARGE_WINDOW", 1_000_000)?,
    })
}

fn window(variable: &str, default: u32) -> Result<u32, String> {
    match std::env::var(variable) {
        Ok(value) => value
            .parse()
            .map_err(|error| format!("{variable}: {error}")),
        Err(_) => Ok(default),
    }
}

/// The fixture as the run uses it: its recorded compactions removed, so that
/// the history is whole again and the run's own passes summarize its start,
/// and the constraint and the open task planted in its first message. The
/// file on disk is never changed.
fn load_fixture() -> Result<Fixture, String> {
    let compressed = std::fs::read(FIXTURE).map_err(|error| format!("{FIXTURE}: {error}"))?;
    let mut json = String::new();
    flate2::read::GzDecoder::new(compressed.as_slice())
        .read_to_string(&mut json)
        .map_err(|error| format!("{FIXTURE}: {error}"))?;
    let mut fixture: Fixture =
        serde_json::from_str(&json).map_err(|error| format!("{FIXTURE}: {error}"))?;
    // The recorded summaries were written before anything but the secrets
    // was planted, so the run starts from the history they replaced.
    fixture.blocks.retain(|block| {
        !matches!(
            block,
            Block::CompactionBoundary(_) | Block::CompactionMarker(_)
        )
    });
    let Some(Block::User(first)) = fixture.blocks.first_mut() else {
        return Err("the fixture does not start with a user message".into());
    };
    let Some(UserContentBlock::Text { text }) = first.content.first_mut() else {
        return Err("the fixture's first message has no text".into());
    };
    if !text.contains(FIRST_MESSAGE_END) {
        return Err(format!(
            "the fixture's first message no longer ends with {FIRST_MESSAGE_END:?}"
        ));
    }
    *text = text.replacen(FIRST_MESSAGE_END, &format!("{PLANTED}{FIRST_MESSAGE_END}"), 1);
    Ok(fixture)
}

/// DeepSeek with the context window `window`: the harness pins the window so
/// that the thresholds are its own.
fn deepseek_model(window: u32) -> ModelSelection {
    let id = std::env::var("DEEPSEEK_FLASH_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    ModelSelection {
        provider_id: "deepseek".into(),
        model: Model {
            name: id.clone(),
            id,
            context_window: window,
            output_limit: None,
            thinking: Vec::new(),
            accepted_extensions: Some(Vec::new()),
        },
        thinking: None,
        service_tier_id: None,
    }
}

fn deepseek() -> Result<Rc<OpenAiProvider>, String> {
    let key =
        std::env::var("DEEPSEEK_API_KEY").map_err(|_| "DEEPSEEK_API_KEY is not set".to_owned())?;
    let api_key = Secret::try_from(key.trim().to_owned()).map_err(|error| error.to_string())?;
    let base =
        std::env::var("DEEPSEEK_BASE_URL").unwrap_or_else(|_| "https://api.deepseek.com/v1".into());
    let base_url = Some(
        reqwest::Url::parse(&base).map_err(|error| format!("DEEPSEEK_BASE_URL: {error}"))?,
    );
    let config = OpenAiConfig {
        api_key: Some(api_key),
        base_url,
        wire: WireApi::ChatCompletions,
        policy: VendorPolicy {
            pass_back_reasoning_content: true,
            ..VendorPolicy::default()
        },
        vendor_id: Some("openai".to_owned()),
        makes_images: false,
    };
    Ok(Rc::new(OpenAiProvider::new(config, Arc::new(SystemClock))))
}

/// One request the provider ran, as the harness recorded it.
struct Request {
    system_prompt: String,
    items: Arc<[InferenceItem]>,
    /// The text of its answer, without reasoning.
    answer: String,
    usage: Option<TokenUsage>,
}

/// Every request of the run, the session copies' included, in order.
type Requests = Rc<RefCell<Vec<Request>>>;

/// A DeepSeek runtime that records each request with its answer and usage.
/// Its fresh runtimes, which the session copy that writes a summary uses,
/// record into the same list.
struct Recorded {
    inner: Box<dyn ProviderRuntime>,
    requests: Requests,
}

impl ProviderRuntime for Recorded {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        let index = {
            let mut requests = self.requests.borrow_mut();
            requests.push(Request {
                system_prompt: request.system_prompt.clone(),
                items: request.items.clone(),
                answer: String::new(),
                usage: None,
            });
            requests.len() - 1
        };
        let requests = self.requests.clone();
        Box::pin(self.inner.run(request).inspect(move |event| {
            let mut requests = requests.borrow_mut();
            let recorded = &mut requests[index];
            match event {
                ProviderEvent::TextDelta(text) => recorded.answer.push_str(text),
                ProviderEvent::Response(usage) => recorded.usage = *usage,
                _ => {}
            }
        }))
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(Self {
            inner: self.inner.fresh(),
            requests: self.requests.clone(),
        })
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        self.inner.close()
    }

    fn request_limits(&self, model: &Model) -> RequestLimits {
        self.inner.request_limits(model)
    }
}

/// The fixture's agents: no Host, so no shell tool runs.
struct NoHosts;

impl HostResolver for NoHosts {
    type Host = NoHost;
}

/// Every runtime is DeepSeek's, recorded, and the conversation opens with
/// the model the run starts from.
struct DeepSeek {
    provider: Rc<OpenAiProvider>,
    http: reqwest::Client,
    selection: ModelSelection,
    requests: Requests,
}

impl ProviderResolver for DeepSeek {
    fn selection<'a>(
        &'a self,
        _root: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<ModelSelection, ResolveError>> {
        let selection = self.selection.clone();
        Box::pin(async move { Ok(selection) })
    }

    fn runtime<'a>(
        &'a self,
        _root: &'a NodeId,
        _model: &'a ModelSelection,
    ) -> LocalBoxFuture<'a, Result<Box<dyn ProviderRuntime>, ResolveError>> {
        let runtime = self
            .provider
            .runtime(RuntimeEnv {
                http: demi_provider_common::ProviderHttp::new(self.http.clone()),
            })
            .map(|inner| {
                Box::new(Recorded {
                    inner,
                    requests: self.requests.clone(),
                }) as Box<dyn ProviderRuntime>
            })
            .map_err(|error| ResolveError::Failed(error.to_string()));
        Box::pin(async move { runtime })
    }

    /// The model's own window: the fixture has no user to limit it, and its
    /// root has no profile.
    fn context_window<'a>(
        &'a self,
        model: &'a ModelSelection,
        _own: Option<u32>,
    ) -> LocalBoxFuture<'a, u32> {
        let window = model.model.context_window;
        Box::pin(async move { window })
    }

    /// DeepSeek speaks the OpenAI family's wire.
    fn family<'a>(
        &'a self,
        _model: &'a ModelSelection,
    ) -> LocalBoxFuture<'a, Result<String, ResolveError>> {
        Box::pin(async { Ok("openai".to_owned()) })
    }

    /// The fixture's user has no profile, so no spawn asks.
    fn profile_selection<'a>(
        &'a self,
        _model: &'a ProfileModel,
    ) -> LocalBoxFuture<'a, Result<ModelSelection, Unavailable>> {
        Box::pin(async { Err(Unavailable::Entry) })
    }
}

/// One turn the harness sent: its question and what the model wrote.
struct Turn {
    question: String,
    thinking: String,
    answer: String,
}

impl Turn {
    /// The transcript excerpt a failed check prints.
    fn excerpt(&self) -> String {
        format!(
            "      user: {}\n      thinking: {}\n      assistant: {}",
            head(&self.question, 200),
            head(&self.thinking, 400),
            self.answer.replace('\n', "\n                 ")
        )
    }
}

/// The summary request behind a new boundary: the recorded request whose
/// answer became the boundary's summary.
enum Summary {
    /// No recorded request answered with the boundary's summary.
    Missing,
    Found {
        usage: Option<TokenUsage>,
        /// The earlier request it repeats, all of it or as a prefix.
        repeats: Option<(usize, bool)>,
    },
}

impl Summary {
    /// The vendor read the repeated request from its cache: DeepSeek reports
    /// its cache, so a repeated request with no cache read is a failure.
    fn cache_ok(&self) -> bool {
        match self {
            Self::Missing => false,
            Self::Found { repeats: None, .. } => true,
            Self::Found { usage, .. } => usage.is_some_and(|usage| usage.cache_read_tokens > 0),
        }
    }
}

/// The fixture opened as a conversation, and what the harness asks it.
struct Conversation {
    server: Rc<AgentServer<NoHosts>>,
    client: TestClient<NoHosts>,
    root: NodeId,
    /// The model the conversation's next request uses.
    model: ModelSelection,
    /// The copies the checks were asked on so far.
    copies: usize,
    /// The `error` blocks the copies ended with.
    copy_errors: usize,
    requests: Requests,
    /// The requests already matched to a boundary.
    matched: HashSet<usize>,
    /// The fixture's tool output, which filler turns quote in turn.
    reference: Vec<String>,
    next_reference: usize,
    fillers: usize,
}

impl Conversation {
    async fn open(fixture: Fixture, model: ModelSelection) -> Result<Self, String> {
        let requests = Requests::default();
        let deepseek = Rc::new(DeepSeek {
            provider: deepseek()?,
            http: reqwest::Client::new(),
            selection: model.clone(),
            requests: requests.clone(),
        });
        let current = model.clone();
        let reference = reference_texts(&fixture.blocks);
        if reference.is_empty() {
            return Err("the fixture has no tool output for the filler turns".into());
        }
        let root = NodeId::try_from("compaction-fixture").expect("the id is not empty");
        let store = MemoryTreeStore::new();
        let block_count = fixture.blocks.len();
        let initial = CheckpointUpdate {
            state: idle_state(model),
            changed_blocks: fixture.blocks.into_iter().enumerate().collect(),
            block_count,
            moved_from: None,
            version: false,
        };
        store
            .create_node(NodeRecord::root(root.clone(), SystemClock.now()), initial)
            .await
            .map_err(|error| error.to_string())?;
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
            instructions: Rc::from(SYSTEM_PROMPT),
            guide: Rc::from(""),
            hosts: Rc::new(NoHosts),
            context: Rc::new([]),
            providers: deepseek,
            shells: Rc::new(NoShells),
            stores: Rc::new(move |_: &NodeId| store.clone() as Rc<dyn AgentTreeStore>),
            clock: Arc::new(SystemClock),
            ids: Rc::new(RandomIds),
            // The product's compaction settings, whatever they are.
            config: ServerConfig::default(),
            // No product shows the conversations' statuses.
            status_changed: Rc::new(|_| {}),
        });
        let client = open_client(&server, &root).await;
        Ok(Self {
            server,
            client,
            root,
            model: current,
            copies: 0,
            copy_errors: 0,
            requests,
            matched: HashSet::new(),
            reference,
            next_reference: 0,
            fillers: 0,
        })
    }

    fn blocks(&self) -> Vec<Block> {
        blocks_of(&self.server, &self.root)
    }

    fn generations(&self) -> usize {
        count(&self.blocks(), |block| {
            matches!(block, Block::CompactionBoundary(_))
        })
    }

    fn errors(&self) -> usize {
        count(&self.blocks(), |block| matches!(block, Block::Error(_)))
    }

    /// The session's estimate of its next request, as the page reads it.
    async fn usage(&self) -> ContextUsage {
        self.server
            .tree(&self.root)
            .expect("the conversation is open")
            .root()
            .session()
            .context_usage()
            .await
            .expect("the fixture's history holds no media to read")
    }

    /// Runs `frame` on the conversation to the end of the action it starts.
    async fn act(&mut self, frame: ClientFrame) {
        act(&mut self.client, frame).await;
    }

    /// Switches the model from the next request on, as the backend does,
    /// and sends the turn it lands with; a compaction the switch needs runs
    /// before that turn's request. Returns that turn.
    async fn switch_to(&mut self, model: ModelSelection) -> Turn {
        let switch = self
            .server
            .prepare_switch(&self.root, model.clone())
            .await
            .expect("DeepSeek's runtime is built")
            .expect("the conversation is open");
        self.server.switch_model(&self.root, switch).await;
        self.model = model;
        self.send(LANDING_PROMPT).await
    }

    /// Sends `text` to the conversation and returns its turn.
    async fn send(&mut self, text: &str) -> Turn {
        send(&self.server, &mut self.client, &self.root, text).await
    }

    /// Asks `text` on a throwaway copy of the conversation as it stands, and
    /// returns its turn: the conversation's history never holds a check.
    /// The copy is stored as a fork's seed is, with the whole transcript
    /// rather than a fork's cut through a completed answer, which would
    /// leave out the usage of the latest request and a boundary after it.
    async fn ask(&mut self, text: &str) -> Turn {
        self.copies += 1;
        let copy = NodeId::try_from(format!("{}-check-{}", self.root, self.copies))
            .expect("the id is not empty");
        let seed = Checkpoint {
            state: idle_state(self.model.clone()),
            transcript: self.blocks(),
            versions: Default::default(),
        };
        self.server
            .initialize_fork(&copy, seed)
            .await
            .expect("an idle copy is stored");
        let mut client = open_client(&self.server, &copy).await;
        let turn = send(&self.server, &mut client, &copy, text).await;
        let blocks = blocks_of(&self.server, &copy);
        self.copy_errors += count(&blocks, |block| matches!(block, Block::Error(_)));
        let passes = count(&blocks, |block| {
            matches!(block, Block::CompactionBoundary(_))
        }) - self.generations();
        if passes > 0 {
            println!("   note: the copy for {:?} compacted {passes} times before its answer", head(text, 20));
        }
        // Closing the copy's only connection disposes its tree.
        client.send(ClientFrame::Close {}).await;
        turn
    }

    /// One filler turn: reference text from the fixture's own tool output,
    /// which the model is asked only to acknowledge. Returns its turn.
    async fn grow(&mut self) -> Turn {
        let mut text = format!("{FILLER_PROMPT}\n\n参考资料 #{}\n", self.fillers);
        while text.len() < FILLER_CHARS {
            let piece = &self.reference[self.next_reference % self.reference.len()];
            self.next_reference += 1;
            text.push_str(piece);
            text.push('\n');
        }
        self.fillers += 1;
        self.send(&text).await
    }

    /// The summary requests behind the boundaries the transcript gained
    /// since it had `before`, each with its usage and the earlier request
    /// it repeats; printed as they are found.
    fn summaries(&mut self, before: usize) -> Vec<Summary> {
        let blocks = self.blocks();
        let summaries: Vec<String> = blocks
            .iter()
            .filter_map(|block| match block {
                Block::CompactionBoundary(boundary) => Some(boundary.summary.clone()),
                _ => None,
            })
            .skip(before)
            .collect();
        let requests = self.requests.borrow();
        let mut found = Vec::new();
        for summary in summaries {
            let request = (0..requests.len()).rev().find(|index| {
                !self.matched.contains(index) && requests[*index].answer.trim() == summary
            });
            let Some(index) = request else {
                println!("   summary request FAILED: no recorded request answered with the summary");
                found.push(Summary::Missing);
                continue;
            };
            self.matched.insert(index);
            let request = &requests[index];
            let repeats = repeated(&requests[..index], request);
            let summary = Summary::Found {
                usage: request.usage,
                repeats,
            };
            let usage = request.usage.unwrap_or_default();
            let read = usage.cache_read_tokens;
            let total = usage.input_tokens + read + usage.cache_write_tokens;
            let covered = if total == 0 { 0 } else { read * 100 / total };
            let repeated = match repeats {
                Some((earlier, true)) => format!("repeats request {earlier} whole"),
                Some((earlier, false)) => format!("repeats a prefix of request {earlier}"),
                None => "repeats no earlier request".into(),
            };
            println!(
                "   summary request {index}: input {}, cache read {read} ({covered}%), cache write {}, output {}; {repeated}{}",
                usage.input_tokens,
                usage.cache_write_tokens,
                usage.output_tokens,
                if summary.cache_ok() {
                    ""
                } else {
                    " — FAILED: no cache read"
                }
            );
            found.push(summary);
        }
        found
    }

    /// The latest summary, which a continuation must not restate.
    fn latest_summary(&self) -> Option<String> {
        self.blocks().iter().rev().find_map(|block| match block {
            Block::CompactionBoundary(boundary) => Some(boundary.summary.clone()),
            _ => None,
        })
    }

    /// Whether every boundary after the first `before` was written by a
    /// model of `window`.
    fn written_with(&self, before: usize, window: u32) -> bool {
        self.blocks()
            .iter()
            .filter_map(|block| match block {
                Block::CompactionBoundary(boundary) => Some(boundary.model.model.context_window),
                _ => None,
            })
            .skip(before)
            .all(|written| written == window)
    }
}

/// A client attached to the conversation `root`, opened. A turn of the
/// real model may take minutes.
async fn open_client(
    server: &Rc<AgentServer<NoHosts>>,
    root: &NodeId,
) -> TestClient<NoHosts> {
    let mut client = TestClient::connect(server, root).with_hang_guard(TURN_TIMEOUT);
    client
        .send(ClientFrame::Open {
            from: None,
            edge: None,
        })
        .await;
    client
        .next_until(|frame| matches!(frame, ServerFrame::PendingSteers { .. }))
        .await;
    client
}

/// An idle session's state with `model`, with nothing waiting.
fn idle_state(model: ModelSelection) -> CheckpointState {
    CheckpointState {
        phase: SessionPhase::Idle,
        queue: Vec::new(),
        agent_inputs: Vec::new(),
        reports: Vec::new(),
        next_turn_reports: Vec::new(),
        intervals: Vec::new(),
        model,
        edits: Vec::new(),
        account: None,
    }
}

fn blocks_of(server: &AgentServer<NoHosts>, root: &NodeId) -> Vec<Block> {
    server
        .tree(root)
        .expect("the conversation is open")
        .root()
        .session()
        .transcript()
        .blocks
}

/// Runs `frame` to the end of the action it starts; a wait longer than an
/// action may take fails as a hang.
async fn act(client: &mut TestClient<NoHosts>, frame: ClientFrame) {
    client.send(frame).await;
    let busy = |frame: &ServerFrame| matches!(frame, ServerFrame::Phase { subagent_id: None, phase } if *phase != SessionPhase::Idle);
    let idle = |frame: &ServerFrame| matches!(frame, ServerFrame::Phase { subagent_id: None, phase } if *phase == SessionPhase::Idle);
    client.next_until(busy).await;
    client.next_until(idle).await;
}

/// Sends `text` to the conversation `root` and returns its turn: the
/// model's text and reasoning after the message, wherever a compaction put
/// the message.
async fn send(
    server: &AgentServer<NoHosts>,
    client: &mut TestClient<NoHosts>,
    root: &NodeId,
    text: &str,
) -> Turn {
    let id = TurnId::try_from(uuid()).expect("a uuid is not empty");
    act(
        client,
        ClientFrame::Send {
            message_id: id.clone(),
            content: client_text(text),
        },
    )
    .await;
    let blocks = blocks_of(server, root);
    let start = blocks
        .iter()
        .position(|block| matches!(block, Block::User(user) if user.turn_id == id))
        .map_or(blocks.len(), |position| position + 1);
    let mut turn = Turn {
        question: text.to_owned(),
        thinking: String::new(),
        answer: String::new(),
    };
    for block in &blocks[start..] {
        match block {
            Block::Text(text) => push_line(&mut turn.answer, &text.text),
            Block::Thinking(thinking) => push_line(&mut turn.thinking, &thinking.text),
            _ => {}
        }
    }
    turn
}

/// The earlier request `request` repeats, apart from the instruction it
/// ends with: its index, and whether it repeats all of it rather than a
/// prefix. The vendor's cache holds what an earlier request sent.
fn repeated(earlier: &[Request], request: &Request) -> Option<(usize, bool)> {
    let (_, body) = request.items.split_last()?;
    earlier
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, other)| other.system_prompt == request.system_prompt)
        .find(|(_, other)| other.items.len() >= body.len() && other.items[..body.len()] == *body)
        .map(|(index, other)| (index, other.items.len() == body.len()))
}

/// The fixture's tool output, as text the filler turns quote.
fn reference_texts(blocks: &[Block]) -> Vec<String> {
    blocks
        .iter()
        .filter_map(|block| match block {
            Block::ToolCall(call) => Some(&call.output),
            _ => None,
        })
        .flatten()
        .filter_map(|part| match part {
            ToolResultContentBlock::Text { text } if !text.is_empty() => Some(text.clone()),
            _ => None,
        })
        .collect()
}

fn count(blocks: &[Block], matches: impl Fn(&Block) -> bool) -> usize {
    blocks.iter().filter(|block| matches(block)).count()
}

fn push_line(into: &mut String, text: &str) {
    if !into.is_empty() {
        into.push('\n');
    }
    into.push_str(text);
}

fn head(text: &str, chars: usize) -> String {
    let flat = text.replace('\n', " ");
    match flat.char_indices().nth(chars) {
        Some((offset, _)) => format!("{}…", &flat[..offset]),
        None => flat,
    }
}

fn uuid() -> String {
    demi_agent_transcript::IdSource::next_id(&RandomIds)
}

/// How many of the secrets `answer` names, with or without their hyphen.
fn recalled(answer: &str) -> usize {
    let answer = answer.to_uppercase();
    SECRETS
        .iter()
        .filter(|(word, digit)| {
            answer.contains(&format!("{word}-{digit}"))
                || answer.contains(&format!("{word}{digit}"))
        })
        .count()
}

/// The answer without the Markdown and space it may open with.
fn opening(answer: &str) -> String {
    answer
        .trim_start_matches(|c: char| c.is_whitespace() || "#*_`>-:：".contains(c))
        .to_lowercase()
}

/// The model kept the constraint: it would ask first, and names the
/// directory.
fn keeps_constraint(answer: &str) -> bool {
    opening(answer).starts_with("ask") && answer.contains("migrations")
}

/// Why `answer`, the first after a pass, restates the summary, or none.
fn restates(answer: &str, summary: &str) -> Option<&'static str> {
    const OPENINGS: [&str; 8] = [
        "summary",
        "here is a summary",
        "here's a summary",
        "conversation summary",
        "总结",
        "摘要",
        "回顾",
        "前情提要",
    ];
    let opening = opening(answer);
    if OPENINGS.iter().any(|start| opening.starts_with(start)) {
        return Some("it opens as a summary");
    }
    if answer.contains("<summary>") {
        return Some("it quotes the summary's tags");
    }
    // Mostly a recap: most of its substantial lines are the summary's own.
    let lines: Vec<&str> = answer
        .lines()
        .map(str::trim)
        .filter(|line| line.chars().count() >= 12)
        .collect();
    let copied = lines.iter().filter(|line| summary.contains(**line)).count();
    (copied >= 2 && copied * 2 > lines.len()).then_some("most of its lines are the summary's")
}

/// What one step did, and what its checks found.
struct Row {
    label: String,
    generations: usize,
    /// The boundaries the step added.
    passes: usize,
    /// Whether the step must compact; one that must not fails with any pass.
    expect_pass: bool,
    /// Whether the passes were written by the model before the switch, for
    /// a switch to a smaller window.
    by_previous_model: Option<bool>,
    summaries: Vec<Summary>,
    /// None for a step that failed before its checks.
    checks: Option<Checks>,
}

/// The answers to the questions after a step.
struct Checks {
    /// Whether the first answer did not restate the summary; none when the
    /// step wrote no summary.
    continuation: Option<bool>,
    next: bool,
    constraint: bool,
    recall: usize,
}

impl Row {
    fn compaction_ok(&self) -> bool {
        (self.passes > 0) == self.expect_pass && self.by_previous_model != Some(false)
    }

    fn passed(&self) -> bool {
        self.compaction_ok()
            && self.summaries.iter().all(Summary::cache_ok)
            && self.checks.as_ref().is_none_or(Checks::passed)
    }
}

impl Checks {
    fn passed(&self) -> bool {
        self.continuation != Some(false) && self.next && self.constraint && self.recall == 3
    }
}

/// The checks after a step that began with `before` boundaries, each asked
/// on its own copy of the conversation: what next, the constraint, and the
/// secrets. `landed` is the conversation's own turn that ran a pass, when
/// one did: a switch's landing turn or a filler turn. That turn's answer,
/// and the answer to what next, the first question after the pass, must not
/// restate a summary the step wrote. After a switch from the window
/// `previous`, the step's passes must be that model's.
async fn check(
    conversation: &mut Conversation,
    label: String,
    before: usize,
    expect_pass: bool,
    landed: Option<&Turn>,
    previous: Option<u32>,
) -> Row {
    let generations = conversation.generations();
    let passes = generations - before;
    let by_previous_model = previous.map(|window| conversation.written_with(before, window));
    let summaries = conversation.summaries(before);
    let summary = conversation.latest_summary().filter(|_| passes > 0);

    let next = conversation.ask(NEXT_PROMPT).await;
    let continuation = summary.as_deref().map(|summary| {
        let mut kept = true;
        for turn in landed.into_iter().chain([&next]) {
            if let Some(reason) = restates(&turn.answer, summary) {
                println!("   continuation FAILED: {reason}");
                println!("{}", turn.excerpt());
                kept = false;
            }
        }
        kept
    });
    if let Some(turn) = landed {
        println!("   landing turn: {}", head(&turn.answer, 120));
    }
    println!("   what next: {}", head(&next.answer, 300));
    let names_task = next.answer.contains(OPEN_TASK);
    if !names_task {
        println!("   open task FAILED: the answer does not name {OPEN_TASK}");
        println!("{}", next.excerpt());
    }

    let constraint = conversation.ask(CONSTRAINT_PROMPT).await;
    let keeps = keeps_constraint(&constraint.answer);
    println!("   constraint: {}", head(&constraint.answer, 200));
    if !keeps {
        println!("   constraint FAILED: the answer does not ASK about migrations/");
        println!("{}", constraint.excerpt());
    }

    let recall = conversation.ask(RECALL_PROMPT).await;
    let secrets = recalled(&recall.answer);
    println!("   recall {secrets}/3: {}", head(&recall.answer, 160));
    if secrets < 3 {
        println!("{}", recall.excerpt());
    }

    Row {
        label,
        generations,
        passes,
        expect_pass,
        by_previous_model,
        summaries,
        checks: Some(Checks {
            continuation,
            next: names_task,
            constraint: keeps,
            recall: secrets,
        }),
    }
}

/// Filler turns until the session admits a `compact` or compacts by itself;
/// the filler turn that compacted, when one did.
async fn fill_to_compact(conversation: &mut Conversation) -> Result<Option<Turn>, String> {
    let before = conversation.generations();
    for _ in 0..MAX_FILLER_TURNS {
        if conversation.usage().await.admits_compaction() {
            return Ok(None);
        }
        let turn = conversation.grow().await;
        if conversation.generations() > before {
            return Ok(Some(turn));
        }
    }
    Err(format!(
        "{MAX_FILLER_TURNS} filler turns did not fill the context"
    ))
}

/// Filler turns until the estimate reaches `tokens`, or until one of them
/// compacts.
async fn fill_to(conversation: &mut Conversation, tokens: u64) -> Result<(), String> {
    let before = conversation.generations();
    for _ in 0..MAX_FILLER_TURNS {
        if conversation.generations() > before || conversation.usage().await.tokens >= tokens {
            return Ok(());
        }
        conversation.grow().await;
    }
    Err(format!(
        "{MAX_FILLER_TURNS} filler turns did not reach {tokens} tokens"
    ))
}

/// The fixture's size as it was built: its tokens and its compactions.
struct Built(u64, u32);

impl Built {
    fn of(fixture: &Fixture) -> Self {
        Self(fixture.built_tokens, fixture.generations)
    }
}

/// What the run starts from. The size of the history shows in the first
/// request's usage: until then the session's estimate rests on the usage the
/// fixture recorded after its last compaction, which the run removed.
fn loaded(conversation: &Conversation, built: Built, run: &Run) {
    println!(
        "loaded fixture (built over ≈{} tokens and {} compactions): {} blocks with its recorded compactions removed; windows small={} large={}, passes={}",
        built.0,
        built.1,
        conversation.blocks().len(),
        run.small,
        run.large,
        run.passes
    );
}

/// Baseline on the large window, then `passes` compactions on the small
/// one: the first by the switch to it, each later one forced with `compact`
/// once filler turns made the session admit it, or by the session itself
/// when the filler reached its threshold first.
async fn recall(fixture: Fixture, run: &Run) -> Result<bool, String> {
    let built = Built::of(&fixture);
    let mut conversation = Conversation::open(fixture, deepseek_model(run.large)).await?;
    loaded(&conversation, built, run);

    println!("\n── baseline on the large window, expecting no compaction");
    let mut rows = vec![check(&mut conversation, "baseline".into(), 0, false, None, None).await];

    for pass in 1..=run.passes {
        let before = conversation.generations();
        let (trigger, landed) = if pass == 1 {
            println!("\n── pass {pass}/{}: switch to the small window", run.passes);
            let landed = conversation.switch_to(deepseek_model(run.small)).await;
            ("switch", Some(landed))
        } else {
            println!("\n── pass {pass}/{}: filler, then compact", run.passes);
            match fill_to_compact(&mut conversation).await? {
                Some(landed) => ("auto", Some(landed)),
                None => {
                    conversation.act(ClientFrame::Compact { node: None }).await;
                    ("compact", None)
                }
            }
        };
        let label = format!("pass {pass} ({trigger})");
        let row = check(&mut conversation, label, before, true, landed.as_ref(), None).await;
        let compacted = row.passes > 0;
        rows.push(row);
        if !compacted {
            println!("   no compaction: stopping");
            break;
        }
    }
    Ok(report("LONG-SESSION COMPACTION VERIFY", &conversation, &rows))
}

/// Each pass switches to the large window, which must not compact, fills it
/// past the small window, and switches back, which must compact with the
/// large window's model first.
async fn switch(fixture: Fixture, run: &Run) -> Result<bool, String> {
    let built = Built::of(&fixture);
    let (small, large) = (deepseek_model(run.small), deepseek_model(run.large));
    let mut conversation = Conversation::open(fixture, small.clone()).await?;
    loaded(&conversation, built, run);

    let mut rows = Vec::new();
    for pass in 1..=run.passes {
        println!(
            "\n── pass {pass}/{}: switch small → large, expecting no compaction",
            run.passes
        );
        let before = conversation.generations();
        let landed = conversation.switch_to(large.clone()).await;
        if pass == 1 {
            let label = "small → large".to_owned();
            rows.push(check(&mut conversation, label, before, false, Some(&landed), None).await);
        }
        fill_to(&mut conversation, u64::from(run.small)).await?;
        let grown = conversation.generations() - before;
        if grown > 0 {
            println!("   FAILED: the large window compacted {grown} times");
            rows.push(Row {
                label: format!("pass {pass} on large"),
                generations: conversation.generations(),
                passes: grown,
                expect_pass: false,
                by_previous_model: None,
                summaries: conversation.summaries(before),
                checks: None,
            });
            break;
        }

        println!(
            "\n── pass {pass}/{}: switch large → small, expecting a compaction by the large window's model",
            run.passes
        );
        let before = conversation.generations();
        let landed = conversation.switch_to(small.clone()).await;
        let label = format!("pass {pass} large → small");
        let row = check(
            &mut conversation,
            label,
            before,
            true,
            Some(&landed),
            Some(run.large),
        )
        .await;
        let compacted = row.passes > 0;
        rows.push(row);
        if !compacted {
            println!("   no compaction: stopping");
            break;
        }
    }
    Ok(report("WINDOW-SWITCH COMPACTION VERIFY", &conversation, &rows))
}

/// Prints the run's table, and answers whether it passed: every step's
/// checks, and no `error` block.
fn report(title: &str, conversation: &Conversation, rows: &[Row]) -> bool {
    let errors = conversation.errors() + conversation.copy_errors;
    let mark = |ok: bool| if ok { "ok" } else { "FAIL" };
    println!("\n===== {title} =====");
    println!(
        "{:<24} | gens | passes | cache read / input        | continue | next | constraint | recall",
        "step"
    );
    for row in rows {
        let cache = if row.summaries.is_empty() {
            "—".to_owned()
        } else {
            row.summaries
                .iter()
                .map(|summary| match summary {
                    Summary::Missing => "missing FAIL".to_owned(),
                    Summary::Found { usage, .. } => {
                        let usage = usage.unwrap_or_default();
                        format!(
                            "{}/{} {}",
                            usage.cache_read_tokens,
                            usage.input_tokens + usage.cache_read_tokens + usage.cache_write_tokens,
                            mark(summary.cache_ok())
                        )
                    }
                })
                .collect::<Vec<_>>()
                .join(", ")
        };
        let passes = match (row.compaction_ok(), row.by_previous_model) {
            (true, _) => row.passes.to_string(),
            (false, Some(false)) => format!("{} FAIL: by the new model", row.passes),
            (false, _) => format!("{} FAIL", row.passes),
        };
        let checks = match &row.checks {
            Some(checks) => format!(
                "{:<8} | {:<4} | {:<10} | {}/3{}",
                checks.continuation.map_or("—", mark),
                mark(checks.next),
                mark(checks.constraint),
                checks.recall,
                if checks.recall == 3 { "" } else { " FAIL" }
            ),
            None => format!("{:<8} | {:<4} | {:<10} | —", "—", "—", "—"),
        };
        println!(
            "{:<24} | {:>4} | {:<6} | {:<25} | {checks}",
            row.label, row.generations, passes, cache,
        );
    }
    let usage = conversation.requests.borrow().iter().fold(
        TokenUsage::default(),
        |total, request| {
            let usage = request.usage.unwrap_or_default();
            TokenUsage {
                input_tokens: total.input_tokens + usage.input_tokens,
                output_tokens: total.output_tokens + usage.output_tokens,
                cache_read_tokens: total.cache_read_tokens + usage.cache_read_tokens,
                cache_write_tokens: total.cache_write_tokens + usage.cache_write_tokens,
            }
        },
    );
    println!(
        "requests: {}; tokens: input {} uncached, {} read from cache, {} written to cache, output {}",
        conversation.requests.borrow().len(),
        usage.input_tokens,
        usage.cache_read_tokens,
        usage.cache_write_tokens,
        usage.output_tokens
    );
    println!(
        "checks asked on {} copies; error blocks: {errors}",
        conversation.copies
    );
    let passed = errors == 0 && rows.iter().all(Row::passed);
    println!("{}", if passed { "PASSED" } else { "FAILED" });
    passed
}
