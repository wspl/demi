//! The compaction fixture's harness (`README.md` beside it): it opens the
//! committed large-context session as a conversation of an agent server,
//! talks to the real DeepSeek V4 Flash through the OpenAI-compatible
//! provider, and checks that three secrets planted before the first
//! compaction are still recalled.
//!
//! ```sh
//! cargo run -p demi-agent --features compaction-fixture --example compaction-fixture -- recall
//! cargo run -p demi-agent --features compaction-fixture --example compaction-fixture -- switch
//! ```
//!
//! It calls a real model, so no build of the one selection compiles it and
//! no test runs it.

use std::{io::Read, process::ExitCode, rc::Rc, sync::Arc, time::Duration};

use demi_agent::{
    AgentHarness, AgentServer, AgentTreeStore, CompactionConfig, FailureReader, PromptContext,
    ProviderResolver, RandomIds, ResolveError, ServerConfig, ServerDeps, SessionConfig,
    store::{CheckpointState, CheckpointUpdate, CommandStateSnapshot, NodeRecord},
    testing::{MemoryTreeStore, TestClient, client_text},
    transcript::estimate::context_tokens,
};
use demi_agent_protocol::{ClientFrame, ServerFrame};
use demi_core::{
    Block, Clock, Model, ModelSelection, NodeId, ProviderErrorDiagnostics, ProviderFailureFacts,
    SessionPhase, SystemClock, Timestamp, TurnId, WireApi,
};
use demi_provider::{Provider, ProviderRuntime, RuntimeEnv, Secret};
use demi_provider_openai_api::{OpenAiConfig, OpenAiProvider, VendorPolicy};
use demi_shell::CommandSet;
use futures_util::future::LocalBoxFuture;
use serde::Deserialize;

const FIXTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/fixtures/compaction/large-context-fixture.json.gz"
);
const SYSTEM_PROMPT: &str =
    "You are a careful coding assistant. Remember any secrets the user told you verbatim.";
const RECALL_PROMPT: &str =
    "只回答暗号值,用「ALPHA=…, BETA=…, GAMMA=…」格式:我最早让你记住的三个暗号分别是什么?";
const SECRETS: [(&str, &str); 3] = [("ZEBRA", "7"), ("QUARTZ", "9"), ("NIMBUS", "3")];
const FILLER_PROMPT: &str = "忽略下列填充并只回复 ok。";

/// The recall check's window, and the extra compactions it forces.
const RECALL_WINDOW: u32 = 200_000;
const EXTRA_GENERATIONS: usize = 3;
/// The history each forced compaction keeps; small, so that a little
/// filler gives it something to summarize.
const RECALL_KEEP_RECENT: u64 = 1_000;
const SWITCH_KEEP_RECENT: u64 = 4_000;

/// How long one turn of the real model may take.
const TURN_TIMEOUT: Duration = Duration::from_secs(600);

/// The committed fixture: a session that compacted several times, in the
/// agent's block format.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Fixture {
    harness: String,
    cwd: String,
    built_tokens: u64,
    generations: u32,
    blocks: Vec<Block>,
}

fn main() -> ExitCode {
    let mode = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "recall".to_owned());
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build_local(tokio::runtime::LocalOptions::default())
        .expect("a local runtime starts");
    let passed = runtime.block_on(async {
        let fixture = load_fixture()?;
        match mode.as_str() {
            "recall" => recall(fixture).await,
            "switch" => switch(fixture).await,
            other => Err(format!("unknown mode {other:?}: recall or switch")),
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

fn load_fixture() -> Result<Fixture, String> {
    let compressed = std::fs::read(FIXTURE).map_err(|error| format!("{FIXTURE}: {error}"))?;
    let mut json = String::new();
    flate2::read::GzDecoder::new(compressed.as_slice())
        .read_to_string(&mut json)
        .map_err(|error| format!("{FIXTURE}: {error}"))?;
    serde_json::from_str(&json).map_err(|error| format!("{FIXTURE}: {error}"))
}

/// DeepSeek V4 Flash with the context window `window`: the harness pins the
/// window so that the thresholds are its own.
fn flash(window: u32) -> ModelSelection {
    let id = std::env::var("DEEPSEEK_FLASH_MODEL").unwrap_or_else(|_| "deepseek-v4-flash".into());
    ModelSelection {
        provider_id: "deepseek".into(),
        model: Model {
            id,
            name: "DeepSeek V4 Flash".into(),
            context_window: window,
            input_limit: None,
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
        base.parse()
            .map_err(|error| format!("DEEPSEEK_BASE_URL: {error}"))?,
    );
    let config = OpenAiConfig {
        id: "deepseek".into(),
        display_name: "DeepSeek".into(),
        api_key,
        base_url,
        wire: WireApi::ChatCompletions,
        policy: VendorPolicy {
            pass_back_reasoning_content: true,
            replay_assistant_status: false,
        },
    };
    Ok(Rc::new(OpenAiProvider::new(config, Arc::new(SystemClock))))
}

/// The fixture's harness: its name, no commands, and the system prompt the
/// fixture was built with.
struct FixtureHarness {
    name: String,
}

impl AgentHarness for FixtureHarness {
    fn name(&self) -> &str {
        &self.name
    }

    fn commands(&self) -> Rc<CommandSet> {
        Rc::new(CommandSet::new())
    }

    async fn system_prompt(&self, _context: PromptContext<'_>, _commands: &str) -> String {
        SYSTEM_PROMPT.to_owned()
    }
}

/// Every runtime is DeepSeek's.
struct DeepSeek {
    provider: Rc<OpenAiProvider>,
    http: reqwest::Client,
}

impl ProviderResolver for DeepSeek {
    fn runtime<'a>(
        &'a self,
        _root: &'a NodeId,
        _model: &'a ModelSelection,
    ) -> LocalBoxFuture<'a, Result<Box<dyn ProviderRuntime>, ResolveError>> {
        let runtime = self
            .provider
            .runtime(RuntimeEnv {
                http: self.http.clone(),
            })
            .map_err(|error| ResolveError::Failed(error.to_string()));
        Box::pin(async move { runtime })
    }
}

impl FailureReader for DeepSeek {
    fn read(
        &self,
        _provider: &str,
        diagnostics: &ProviderErrorDiagnostics,
        received_at: Timestamp,
    ) -> Option<ProviderFailureFacts> {
        Some(self.provider.read_failure(diagnostics, received_at))
    }
}

/// The fixture opened as a conversation, and what the harness asks it.
struct Conversation {
    server: Rc<AgentServer<FixtureHarness>>,
    client: TestClient<FixtureHarness>,
    root: NodeId,
}

impl Conversation {
    async fn open(
        fixture: Fixture,
        model: ModelSelection,
        keep_recent: u64,
    ) -> Result<Self, String> {
        let provider = deepseek()?;
        let deepseek = Rc::new(DeepSeek {
            provider,
            http: reqwest::Client::new(),
        });
        let root = NodeId::try_from("compaction-fixture").expect("the id is not empty");
        let store = MemoryTreeStore::new();
        let block_count = fixture.blocks.len();
        let initial = CheckpointUpdate {
            state: CheckpointState {
                phase: SessionPhase::Idle,
                queue: Vec::new(),
                agent_inputs: Vec::new(),
                wakeups: Vec::new(),
                cwd: fixture.cwd.clone(),
                model: model.clone(),
                harness: fixture.harness.clone(),
                edits: Vec::new(),
            },
            command_state: Some(CommandStateSnapshot::initial()),
            changed_blocks: fixture.blocks.into_iter().enumerate().collect(),
            block_count,
        };
        store
            .create_node(NodeRecord::root(root.clone(), SystemClock.now()), initial)
            .await
            .map_err(|error| error.to_string())?;
        let config = ServerConfig {
            session: SessionConfig {
                compaction: CompactionConfig {
                    keep_recent_tokens: keep_recent,
                    threshold_percent: Some(80),
                },
                ..SessionConfig::default()
            },
            ..ServerConfig::default()
        };
        let server = AgentServer::new(ServerDeps {
            harness: Rc::new(FixtureHarness {
                name: fixture.harness,
            }),
            providers: deepseek.clone(),
            failures: deepseek,
            stores: Rc::new(move |_: &NodeId| store.clone() as Rc<dyn AgentTreeStore>),
            clock: Arc::new(SystemClock),
            ids: Rc::new(RandomIds),
            config,
        });
        let mut client = TestClient::connect(&server, &root, &fixture.cwd);
        client.send(ClientFrame::Open { model }).await;
        client
            .next_until(|frame| matches!(frame, ServerFrame::PendingSteers { .. }))
            .await;
        Ok(Self {
            server,
            client,
            root,
        })
    }

    fn blocks(&self) -> Vec<Block> {
        self.server
            .tree(&self.root)
            .expect("the conversation is open")
            .root()
            .session()
            .transcript()
            .blocks
    }

    fn generations(&self) -> usize {
        count(&self.blocks(), |block| {
            matches!(block, Block::CompactionBoundary(_))
        })
    }

    fn errors(&self) -> usize {
        count(&self.blocks(), |block| matches!(block, Block::Error(_)))
    }

    /// The history since the last compaction, as the next request would
    /// estimate it with `window`.
    fn context(&self, window: u32) -> u64 {
        let blocks = self.blocks();
        let start = blocks
            .iter()
            .rposition(|block| matches!(block, Block::CompactionBoundary(_)))
            .unwrap_or(0);
        context_tokens(&blocks[start..], Some(window))
    }

    /// Runs `frame` to the end of the action it starts.
    async fn act(&mut self, frame: ClientFrame) -> Result<(), String> {
        self.client.send(frame).await;
        let busy = |frame: &ServerFrame| matches!(frame, ServerFrame::Phase { phase } if *phase != SessionPhase::Idle);
        let idle = |frame: &ServerFrame| matches!(frame, ServerFrame::Phase { phase } if *phase == SessionPhase::Idle);
        let ended = async {
            self.client.next_until(busy).await;
            self.client.next_until(idle).await
        };
        tokio::time::timeout(TURN_TIMEOUT, ended)
            .await
            .map(|_| ())
            .map_err(|_| "the turn did not end in time".to_owned())
    }

    /// Sends `text` and returns the assistant text of its turn.
    async fn send(&mut self, text: &str) -> Result<String, String> {
        let before = self.blocks().len();
        let id = TurnId::try_from(uuid()).expect("a uuid is not empty");
        self.act(ClientFrame::Send {
            message_id: id,
            content: client_text(text),
        })
        .await?;
        Ok(self.blocks()[before..]
            .iter()
            .filter_map(|block| match block {
                Block::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(" "))
    }

    async fn recall(&mut self) -> Result<usize, String> {
        let answer = self.send(RECALL_PROMPT).await?;
        let recalled = recalled(&answer);
        println!("   recall {recalled}/3: {}", tail(&answer));
        Ok(recalled)
    }

    async fn grow(&mut self, label: &str, chars: usize) -> Result<(), String> {
        let filler = format!("{FILLER_PROMPT}\n\n{label}-{}", "x".repeat(chars));
        self.send(&filler).await.map(|_| ())
    }
}

fn count(blocks: &[Block], matches: impl Fn(&Block) -> bool) -> usize {
    blocks.iter().filter(|block| matches(block)).count()
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

fn tail(answer: &str) -> &str {
    let start = answer
        .char_indices()
        .rev()
        .nth(119)
        .map_or(0, |(offset, _)| offset);
    &answer[start..]
}

fn uuid() -> String {
    demi_agent::IdSource::next_id(&RandomIds)
}

/// Baseline recall, then `EXTRA_GENERATIONS` forced compactions, each
/// followed by recall; it fails once recall drops below 3/3.
async fn recall(fixture: Fixture) -> Result<bool, String> {
    println!(
        "loaded fixture: total≈{} tokens, {} blocks, {} generations (extra={EXTRA_GENERATIONS}, keepRecent={RECALL_KEEP_RECENT})",
        fixture.built_tokens,
        fixture.blocks.len(),
        fixture.generations
    );
    let mut conversation =
        Conversation::open(fixture, flash(RECALL_WINDOW), RECALL_KEEP_RECENT).await?;
    let baseline = conversation.generations();
    println!("\n── baseline recall");
    let mut rows = vec![(conversation.generations(), conversation.recall().await?)];
    for extra in 1..=EXTRA_GENERATIONS {
        if rows.last().is_some_and(|(_, recalled)| *recalled < 3) {
            break;
        }
        println!("\n── extra compact #{extra}/{EXTRA_GENERATIONS}");
        let before = conversation.generations();
        conversation.grow(&format!("VERIFY-{extra}"), 6_000).await?;
        conversation.act(ClientFrame::Compact {}).await?;
        if conversation.generations() <= before {
            println!("   compact added no generation; growing harder and retrying once");
            conversation.grow(&format!("FORCE-{extra}"), 20_000).await?;
            conversation.act(ClientFrame::Compact {}).await?;
        }
        if conversation.generations() <= before {
            println!("   still no new generation after the retry: stopping");
            break;
        }
        println!(
            "   compacted: {before} → {} generations",
            conversation.generations()
        );
        rows.push((conversation.generations(), conversation.recall().await?));
    }
    let errors = conversation.errors();
    println!("\n===== LONG-SESSION COMPACTION VERIFY =====");
    println!("generations | recall");
    for (generations, recalled) in &rows {
        println!("{generations:>11} | {recalled}/3");
    }
    println!("error blocks: {errors}");
    let passed = baseline >= 2 && errors == 0 && rows.iter().all(|(_, recalled)| *recalled == 3);
    println!("{}", if passed { "PASSED" } else { "FAILED" });
    Ok(passed)
}

/// Small → large must not compact; large → small must compact with the
/// model before the switch; the secrets are recalled throughout.
async fn switch(fixture: Fixture) -> Result<bool, String> {
    let small_window = window("COMPACTION_FIXTURE_SMALL_WINDOW", 8_000)?;
    let large_window = window("COMPACTION_FIXTURE_LARGE_WINDOW", 400_000)?;
    let (small, large) = (flash(small_window), flash(large_window));
    let mut conversation = Conversation::open(fixture, small.clone(), SWITCH_KEEP_RECENT).await?;
    println!(
        "loaded: {} blocks, ctx≈{} replayable tokens, {} generations; windows small={small_window} large={large_window}",
        conversation.blocks().len(),
        conversation.context(large_window),
        conversation.generations()
    );

    println!("\n── STEP 1: switch small → large, expecting no compaction");
    let before = conversation.generations();
    conversation.act_switch(large.clone()).await;
    let recall_up = conversation.recall().await?;
    let no_compaction_up = conversation.generations() == before;
    println!("   compacted on the larger window: {}", !no_compaction_up);

    println!("\n── STEP 2: grow until the small window must compact");
    let threshold = u64::from(small_window) * 8 / 10;
    let mut turns = 0;
    while conversation.context(small_window) < threshold
        || conversation.context(large_window) < 2 * SWITCH_KEEP_RECENT
    {
        conversation.grow(&format!("FILLER-{turns}"), 8_000).await?;
        turns += 1;
        if turns > 40 {
            return Err("40 filler turns did not fill the small window".into());
        }
    }
    println!(
        "   ready after {turns} turns: estimate@small={} (threshold {threshold})",
        conversation.context(small_window)
    );

    println!(
        "\n── STEP 3: switch large → small, expecting a compaction by the model before the switch"
    );
    let before = conversation.generations();
    conversation.act_switch(small).await;
    let recall_down = conversation.recall().await?;
    let compacted_down = conversation.generations() > before;
    println!("   compacted for the smaller window: {compacted_down}");

    println!("\n── STEP 4: switch back to large");
    conversation.act_switch(large).await;
    let recall_back = conversation.recall().await?;

    let errors = conversation.errors();
    println!("\n===== WINDOW-SWITCH COMPACTION VERIFY =====");
    println!("step 1  small→large: no compaction = {no_compaction_up}, recall {recall_up}/3");
    println!("step 3  large→small: compaction = {compacted_down}, recall {recall_down}/3");
    println!("step 4  switch back: recall {recall_back}/3");
    println!("error blocks: {errors}");
    let passed = no_compaction_up
        && compacted_down
        && recall_up == 3
        && recall_down == 3
        && recall_back == 3
        && errors == 0;
    println!("{}", if passed { "PASSED" } else { "FAILED" });
    Ok(passed)
}

impl Conversation {
    /// Switches the model from the next turn on.
    async fn act_switch(&mut self, model: ModelSelection) {
        self.client
            .send(ClientFrame::SetProvider { model, apply: None })
            .await;
    }
}

fn window(variable: &str, default: u32) -> Result<u32, String> {
    match std::env::var(variable) {
        Ok(value) => value
            .parse()
            .map_err(|error| format!("{variable}: {error}")),
        Err(_) => Ok(default),
    }
}
