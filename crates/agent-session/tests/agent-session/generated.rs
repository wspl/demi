//! Compaction over generated sessions, the first level of its verification.
//!
//! Each fixed seed drives one session through turns of text, signed reasoning
//! of several vendors, parallel and sequential calls with text and image
//! results, steers and agent messages that arrive during a call, a Stop and
//! Continue, a resume after a restart, messages with images and model switches,
//! some answered by the new model before the pass. It compacts after every
//! step, besides the passes its thresholds start.
//!
//! After every step the test checks:
//!
//! - every request, encoded by every provider's request builder, is one its
//!   vendor takes: roles in order, every result after its call, no call without
//!   its result, signed reasoning only where its vendor accepts it;
//! - every summary request, without its instruction, begins with the latest
//!   request the session sent to the same model since the summary request
//!   before, encoded byte for byte the same by every builder;
//! - each new boundary stands at the cut point the recent work's budget gives,
//!   and keeps the user's words within theirs;
//! - the estimate after each pass is under the threshold, or the pass freed
//!   nothing;
//! - no block left the transcript, but for the leftovers of a stopped request
//!   that a Continue unwinds.
//!
//! A seed that ever failed is kept in [`FIXED_SEEDS`].

use std::collections::HashSet;

use demi_agent_session::{ActionHandle, testing::COMPACTION_SUMMARY_INSTRUCTION, token_threshold};
use demi_agent_store::{
    media::{BlobStore, ModelView},
    testing::{MemoryBlobs, png},
};
use demi_agent_transcript::{
    RequestView,
    estimate::{block_tokens, content_tokens, context_tokens},
};
use demi_provider_common::{
    InferenceRequest, MediaBytes, ProviderRun, RequestLimits, ToolResultKinds,
    VendorPolicy,
};
use demi_shared_types::{FileExtension, KeptMessage, MediaSource};
use futures_util::{StreamExt as _, stream};
use rand::{Rng, SeedableRng, rngs::StdRng};
use serde_json::Value;

use super::*;

/// The seeds every run drives, and those that once found a defect, kept as
/// fixed cases.
const SEEDS: std::ops::Range<u64> = 1..5;
const FIXED_SEEDS: &[u64] = &[];

/// The steps each seed's session takes.
const STEPS: usize = 24;

/// The two models a session switches between: windows of 20,000 and
/// 40,000 tokens, whose thresholds are half of them.
const MODELS: [(&str, u32); 2] = [("gen-a", 20_000), ("gen-b", 40_000)];

/// The user's words a pass keeps, and the most recent work it keeps, in
/// estimated tokens, or the share of the window in use, in percent, when
/// that is less.
const KEPT_WORDS: (u64, u64) = (20_000, 10);
const KEPT_WORK: (u64, u64) = (20_000, 15);

/// Each provider's request builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Builder {
    Anthropic,
    Responses,
    ChatCompletions,
    Google,
    Codex,
    ClaudeCode,
    Grok,
}

pub(super) const BUILDERS: [Builder; 7] = [
    Builder::Anthropic,
    Builder::Responses,
    Builder::ChatCompletions,
    Builder::Google,
    Builder::Codex,
    Builder::ClaudeCode,
    Builder::Grok,
];

impl Builder {
    /// What the provider sends of `request`: its body, or for Claude Code
    /// the session a new process resumes, one entry per line.
    pub(super) fn encode(self, request: &InferenceRequest) -> String {
        let policy = VendorPolicy::default();
        let bytes = match self {
            Self::Anthropic => demi_provider_anthropic_api::testing::body(request, policy),
            Self::Responses => demi_provider_openai_api::testing::responses_body(request, policy),
            Self::ChatCompletions => {
                demi_provider_openai_api::testing::chat_completions_body(request, policy)
            }
            Self::Google => demi_provider_google::testing::body(request),
            Self::Codex => demi_provider_codex::testing::body(request),
            Self::ClaudeCode => {
                demi_provider_claude_code::testing::session(request, "2026-10-11T12:00:00.000Z")
            }
            Self::Grok => demi_provider_grok_build::testing::body(request),
        };
        String::from_utf8(bytes).expect("a request is UTF-8")
    }

    /// The encoding as JSON: the body, or for Claude Code the array of its
    /// session's entries, without the ids that chain them.
    pub(super) fn json(self, request: &InferenceRequest) -> Value {
        let encoded = self.encode(request);
        match self {
            Self::ClaudeCode => Value::Array(
                encoded
                    .lines()
                    .map(|line| {
                        let mut entry: Value = serde_json::from_str(line).expect("an entry is JSON");
                        if let Some(fields) = entry.as_object_mut() {
                            fields.remove("uuid");
                            fields.remove("parentUuid");
                        }
                        entry
                    })
                    .collect(),
            ),
            _ => serde_json::from_str(&encoded).expect("a body is JSON"),
        }
    }
}

/// Whether `request` is a summary request: a user message holds the
/// instruction's text, in whatever form.
pub(super) fn is_summary(request: &InferenceRequest) -> bool {
    request.items.iter().any(|item| match item {
        InferenceItem::UserMessage { content } | InferenceItem::UserSteer { content } => {
            content.iter().any(|part| {
                matches!(part, UserPart::Text(text) if text.contains(COMPACTION_SUMMARY_INSTRUCTION))
            })
        }
        _ => false,
    })
}

/// The items of `request` without the summary instruction, when it is a summary
/// request.
///
/// The instruction is the last text block of its last user message, which holds
/// nothing else when the history ended with something other than the user's
/// input.
fn without_instruction(request: &InferenceRequest) -> Option<Vec<InferenceItem>> {
    let mut items = request.items.to_vec();
    let (InferenceItem::UserMessage { content } | InferenceItem::UserSteer { content }) =
        items.last_mut()?
    else {
        return None;
    };
    if content.last() != Some(&UserPart::Text(COMPACTION_SUMMARY_INSTRUCTION.to_owned())) {
        return None;
    }
    content.pop();
    if content.is_empty() {
        items.pop();
    }
    Some(items)
}

/// What the generated model answers, drawn from the session's seed.
struct Generated {
    rng: StdRng,
    requests: Vec<InferenceRequest>,
    /// The next request of the session runs until it is cancelled.
    hang: bool,
    /// Rounds per turn, to end a turn after a few.
    rounds: Vec<(String, u32)>,
    serial: u64,
}

#[derive(Clone)]
struct Model(Rc<RefCell<Generated>>);

impl Generated {
    fn next(&mut self) -> u64 {
        self.serial += 1;
        self.serial
    }

    /// A signature of one of the vendors, as its provider tags it.
    fn signature(&mut self) -> String {
        let n = self.next();
        let reasoning = || {
            json!({ "type": "reasoning", "id": format!("rs_{n}"), "summary": [], "encrypted_content": format!("enc-{n}") })
                .to_string()
        };
        match self.rng.random_range(0..5) {
            0 => format!("anthropic:sig-{n}"),
            1 => format!("openai:{}", reasoning()),
            2 => format!("codex:{}", reasoning()),
            3 => format!("google:sig-{n}"),
            _ => format!("grok:sig-{n}"),
        }
    }

    /// The events that answer `request`; none for a request that runs until
    /// it is cancelled.
    fn answer(&mut self, request: &InferenceRequest) -> Option<Vec<ProviderEvent>> {
        self.requests.push(request.clone());
        if is_summary(request) {
            let n = self.next();
            let text = event::text(&format!("Summary {n}: {}", "s".repeat(self.rng.random_range(10..400))));
            return Some(match self.rng.random_range(0..20) {
                // A call beside the summary, which never runs.
                0 | 1 => vec![
                    text,
                    event::tool_call(&format!("summary-call-{n}"), "work", json!({})),
                    event::response(0, 0),
                ],
                // A call alone, which fails the summary request once.
                2 => vec![
                    event::tool_call(&format!("summary-call-{n}"), "work", json!({})),
                    event::response(0, 0),
                ],
                _ => vec![text, event::response(0, 0)],
            });
        }
        if std::mem::take(&mut self.hang) {
            return None;
        }
        let rounds = match self.rounds.iter_mut().find(|(turn, _)| *turn == request.turn_id) {
            Some((_, rounds)) => {
                *rounds += 1;
                *rounds
            }
            None => {
                self.rounds.push((request.turn_id.clone(), 1));
                1
            }
        };
        let mut events = Vec::new();
        if self.rng.random_bool(0.4) {
            let n = self.next();
            events.push(ProviderEvent::ThinkingStart);
            events.push(event::thinking(&format!("thinking {n} {}", "t".repeat(self.rng.random_range(0..200)))));
            let signature = self.signature();
            events.push(ProviderEvent::ThinkingSignature(signature));
        }
        let calls = if rounds > 3 || self.rng.random_bool(0.35) {
            0
        } else {
            self.rng.random_range(1..=3)
        };
        if calls == 0 || self.rng.random_bool(0.3) {
            let n = self.next();
            events.push(event::text(&format!("answer {n} {}", "a".repeat(self.rng.random_range(0..1_600)))));
        }
        for _ in 0..calls {
            let n = self.next();
            events.push(event::tool_call(&format!("call-{n}"), "work", json!({ "n": n })));
        }
        // Now and then the usage reaches the model's threshold, and the turn
        // compacts after the round.
        let window = MODELS
            .iter()
            .find(|(id, _)| *id == request.model_id)
            .map_or(100_000, |(_, window)| u64::from(*window));
        let usage = if self.rng.random_bool(0.15) {
            token_threshold(window, None)
        } else {
            0
        };
        events.push(event::response(usage, 1));
        Some(events)
    }
}

impl ProviderRuntime for Model {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        let answer = self.0.borrow_mut().answer(&request);
        let events = match answer {
            Some(events) => stream::iter(events).boxed_local(),
            None => stream::pending().boxed_local(),
        };
        events
            .take_until(request.cancel.cancelled_owned())
            .boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(self.clone())
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {})
    }

    fn request_limits(&self, _model: &demi_shared_types::Model) -> RequestLimits {
        RequestLimits {
            tool_results: ToolResultKinds::ALL,
            ..RequestLimits::default()
        }
    }
}

/// A generated model of `id`, which reads PNG images.
fn model(id: &str) -> ModelSelection {
    let (_, window) = MODELS
        .iter()
        .find(|(known, _)| *known == id)
        .expect("a generated model");
    let mut model = model_reading("stub", id, &[FileExtension::Png]);
    model.model.context_window = *window;
    model
}

/// What one seed's session shares with its tools.
struct Shared {
    rng: RefCell<StdRng>,
    session: RefCell<Option<AgentSession>>,
    serial: Cell<u64>,
    /// Every image the session was given, held for the estimates.
    images: RefCell<HeldMedia>,
}

impl Shared {
    fn next(&self) -> u64 {
        self.serial.set(self.serial.get() + 1);
        self.serial.get()
    }
}

/// The tool every call names: it returns text of a drawn size, or an image,
/// and during some calls a steer or an agent message arrives.
fn work_tool(shared: &Rc<Shared>) -> (String, Invoke) {
    let shared = shared.clone();
    tool("work", move |_| {
        let shared = shared.clone();
        Box::pin(async move {
            let (arrival, image, size) = {
                let mut rng = shared.rng.borrow_mut();
                (rng.random_range(0..10), rng.random_bool(0.15), rng.random_range(10..3_000))
            };
            let session = shared.session.borrow().clone();
            if let Some(session) = session {
                let n = shared.next();
                match arrival {
                    0 => {
                        let _ = session.steer(text(&format!("steer {n}")), BlockId::try_from(format!("steer-{n}")).unwrap());
                    }
                    1 => {
                        let _ = session.accept_agent_message(agent_message(&format!("message-{n}"))).await;
                    }
                    _ => {}
                }
            }
            if image {
                let shot = png(3, 2, (shared.next() % 200) as u8);
                shared.images.borrow_mut().hold(BlobRef::of(shot.as_bytes()), shot.clone());
                return Ok(ToolOutcome {
                    output: vec![ResultPart::Image(MediaBytes {
                        data: shot,
                        media_type: "image/png".into(),
                    })],
                    ..output("")
                });
            }
            Ok(output(&"o".repeat(size)))
        })
    })
}

/// One seed's driver: its session, its store, what it has checked.
struct Driver {
    seed: u64,
    model: Model,
    shared: Rc<Shared>,
    store: Rc<MemoryTreeStore>,
    blobs: Rc<MemoryBlobs>,
    session: AgentSession,
    restarts: u64,
    /// How many requests the model had when the last restart took the
    /// store: a turn the dead process never saved is no part of the
    /// restored history, so its requests are no summary's prefix.
    restarted_at: usize,
    /// The requests checked so far.
    checked: usize,
    /// The boundaries checked so far, by id.
    boundaries: HashSet<BlockId>,
    /// Every block seen that only an edit could remove.
    seen: Vec<BlockId>,
    /// How the histories of the summary requests checked so far ended.
    history_ends: HashSet<HistoryEnd>,
}

impl Driver {
    async fn new(seed: u64) -> Self {
        let model = Model(Rc::new(RefCell::new(Generated {
            rng: StdRng::seed_from_u64(seed),
            requests: Vec::new(),
            hang: false,
            rounds: Vec::new(),
            serial: 0,
        })));
        let shared = Rc::new(Shared {
            rng: RefCell::new(StdRng::seed_from_u64(seed.wrapping_mul(7919))),
            session: RefCell::new(None),
            serial: Cell::new(0),
            images: RefCell::new(HeldMedia::default()),
        });
        let blobs = MemoryBlobs::new();
        let store = MemoryTreeStore::with_blobs(blobs.clone());
        let session = start_with(
            Box::new(model.clone()),
            test_runtime(vec![work_tool(&shared)]),
            &store,
            SessionConfig::default(),
            Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
        )
        .await;
        session
            .update_model(ModelSwitch {
                model: Box::new(self::model("gen-a")),
                runtime: None,
            })
            .unwrap();
        shared.session.replace(Some(session.clone()));
        Self {
            seed,
            model,
            shared,
            store,
            blobs,
            session,
            restarts: 0,
            restarted_at: 0,
            checked: 0,
            boundaries: HashSet::new(),
            seen: Vec::new(),
            history_ends: HashSet::new(),
        }
    }

    fn draw(&self, range: std::ops::Range<u32>) -> u32 {
        self.shared.rng.borrow_mut().random_range(range)
    }

    /// A short message of the user's, with an image now and then.
    async fn message(&self) -> Vec<UserContentBlock> {
        let n = self.shared.next();
        let words = self.draw(1..40) as usize;
        let mut content = text(&format!("message {n} {}", "m".repeat(words * 4)));
        if self.draw(0..5) == 0 {
            let shot = png(4, 3, (n % 200) as u8);
            let blob = self.blobs.put(shot.clone()).await.unwrap();
            self.shared.images.borrow_mut().hold(blob.clone(), shot);
            content.push(UserContentBlock::Image {
                source: MediaSource::Ref {
                    r#ref: blob,
                    media_type: "image/png".into(),
                    width: None,
                    height: None,
                },
            });
        }
        content
    }

    /// Waits for an action, and resumes the turn a failure left
    /// unfinished.
    async fn finish(&self, handle: Result<ActionHandle, AdmissionError>) {
        let Ok(handle) = handle else {
            return;
        };
        if handle.await.is_err() {
            if let Ok(resumed) = self.session.resume(None) {
                let _ = resumed.await;
            }
        }
        self.session.settled().await;
    }

    /// Sends a message whose first request runs until the session stops it.
    async fn send_hanging(&self) -> ActionHandle {
        let before = self.model.0.borrow().requests.len();
        self.model.0.borrow_mut().hang = true;
        let n = self.shared.next();
        let handle = self
            .session
            .send(self.message().await, turn(&format!("t{n}")))
            .unwrap();
        // The hanging request, after any pass before it.
        until(|| {
            let generated = self.model.0.borrow();
            !generated.hang && generated.requests.len() > before
        })
        .await;
        handle
    }

    /// Restores the session from what its store holds now, as a process
    /// that died at this moment left it; a turn it interrupted is recorded
    /// as such and resumed.
    async fn restart(&mut self) {
        let copy = self.store.copy();
        self.restarts += 1;
        self.restarted_at = self.model.0.borrow().requests.len();
        let deps = SessionDeps {
            runtime: Rc::new(test_runtime(vec![work_tool(&self.shared)])),
            store: copy.session_store(&root()),
            ids: Rc::new(SequentialIds::new(&format!("r{}", self.restarts))),
            clock: Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
            config: SessionConfig::default(),
        };
        let (restored, continuation) = AgentSession::restore(
            copy.checkpoint(&root()).unwrap(),
            root(),
            Box::new(self.model.clone()),
            deps,
            &[],
        )
        .await.unwrap();
        let old = std::mem::replace(&mut self.session, restored);
        old.abort().await;
        let _ = old.dispose().await;
        self.store = copy;
        self.shared.session.replace(Some(self.session.clone()));
        if continuation.interrupted {
            self.finish(self.session.resume(None)).await;
        }
    }

    /// One drawn step, then a `compact` action.
    async fn step(&mut self) {
        match self.draw(0..10) {
            0..=3 => {
                let n = self.shared.next();
                let message = self.message().await;
                self.finish(self.session.send(message, turn(&format!("t{n}")))).await;
            }
            4 => {
                let running = self.send_hanging().await;
                self.session.abort().await;
                let _ = running.await;
                self.finish(self.session.resume(None)).await;
            }
            5 => {
                let running = self.send_hanging().await;
                drop(running);
                self.restart().await;
            }
            6 => {
                let n = self.shared.next();
                let _ = self.session.accept_agent_message(agent_message(&format!("idle-{n}"))).await;
                self.session.settled().await;
            }
            // Messages the model answers, then a switch, which lands at the
            // next request; half the time the new model answers a message
            // too before the step's pass, which then cuts only where it
            // answered, and otherwise where the model before it did.
            7 => {
                for _ in 0..self.draw(1..3) {
                    let n = self.shared.next();
                    let message = self.message().await;
                    self.finish(self.session.send(message, turn(&format!("t{n}")))).await;
                }
                let next = if self.session.model().model.id == "gen-a" { "gen-b" } else { "gen-a" };
                let _ = self.session.update_model(ModelSwitch {
                    model: Box::new(model(next)),
                    runtime: None,
                });
                if self.draw(0..2) == 0 {
                    let n = self.shared.next();
                    let message = self.message().await;
                    self.finish(self.session.send(message, turn(&format!("t{n}")))).await;
                }
            }
            8 => self.restart().await,
            // A Stop before any answer, left as it is: the step's pass
            // asks with the stopped message last.
            _ => {
                let running = self.send_hanging().await;
                self.session.abort().await;
                let _ = running.await;
            }
        }
        self.finish(self.session.compact()).await;
    }

    /// What `blocks` weigh for `model`, each as a request of it carries it.
    fn view<'a>(&self, view: &'a ModelView, model: &'a demi_shared_types::Model) -> RequestView<'a> {
        RequestView::new(
            view,
            model,
            RequestLimits {
                tool_results: ToolResultKinds::ALL,
                ..RequestLimits::default()
            },
            &[],
        )
    }

    fn model_view(&self, start: usize, blocks: &[Block]) -> ModelView {
        ModelView::of(start, blocks, &self.shared.images.borrow()).expect("every image is held")
    }

    /// Checks what changed since the last check.
    async fn check(&mut self, step: usize) {
        let context = format!("seed {} step {step}", self.seed);
        let blocks = self.session.history().await.unwrap().blocks;
        self.check_kept_blocks(&blocks, &context);
        let boundaries: Vec<usize> = blocks
            .iter()
            .enumerate()
            .filter(|(_, block)| matches!(block, Block::CompactionBoundary(_)))
            .map(|(index, _)| index)
            .collect();
        for index in boundaries {
            if self.boundaries.insert(blocks[index].id().clone()) {
                self.check_pass(&blocks, index, &context);
            }
        }
        let requests = self.model.0.borrow().requests.clone();
        for (index, request) in requests.iter().enumerate().skip(self.checked) {
            let context = format!("{context} request {index}");
            for builder in BUILDERS {
                valid(builder, request).unwrap_or_else(|problem| {
                    panic!("{context}: {builder:?}: {problem}\n{}", builder.encode(request))
                });
            }
            if is_summary(request) {
                let end = check_summary(&requests[self.restarted_at.min(index)..index], request, &context);
                self.history_ends.insert(end);
            }
        }
        self.checked = requests.len();
    }

    /// No block that only an edit could remove left the transcript: the
    /// leftovers of a stopped request, reasoning, error records and blank
    /// text, may go when a Continue unwinds them.
    fn check_kept_blocks(&mut self, blocks: &[Block], context: &str) {
        let ids: HashSet<&BlockId> = blocks.iter().map(Block::id).collect();
        for id in &self.seen {
            assert!(ids.contains(id), "{context}: block {id} left the transcript");
        }
        self.seen = blocks
            .iter()
            .filter(|block| {
                !matches!(
                    block,
                    Block::Thinking(_) | Block::RedactedThinking(_) | Block::Error(_) | Block::Abort(_)
                ) && !matches!(block, Block::Text(text) if text.text.trim().is_empty())
            })
            .map(|block| block.id().clone())
            .collect();
    }

    /// The boundary at `index` in `blocks`: where it stands, what it keeps,
    /// and what the pass that inserted it freed.
    fn check_pass(&self, blocks: &[Block], index: usize, context: &str) {
        let Block::CompactionBoundary(boundary) = &blocks[index] else {
            unreachable!();
        };
        let Some(marker) = blocks.iter().position(
            |block| matches!(block, Block::CompactionMarker(marker) if marker.boundary_id == boundary.id),
        ) else {
            panic!("{context}: the boundary has no marker");
        };
        let model = &boundary.model.model;
        let window = u64::from(model.context_window);
        let budget = KEPT_WORK.0.min(window * KEPT_WORK.1 / 100);
        let words_budget = KEPT_WORDS.0.min(window * KEPT_WORDS.1 / 100);
        // The history as the pass found it: without the boundary, up to the
        // marker.
        let before: Vec<Block> = blocks[..marker]
            .iter()
            .enumerate()
            .filter(|(at, _)| *at != index)
            .map(|(_, block)| block.clone())
            .collect();
        let start = before
            .iter()
            .rposition(|block| matches!(block, Block::CompactionBoundary(_)))
            .unwrap_or(0);
        let view = self.model_view(start, &before[start..]);
        let request = self.view(&view, model);
        let weigh = |from: usize| -> u64 {
            before[from..]
                .iter()
                .map(|block| block_tokens(block, &request))
                .sum()
        };
        // The cut: the latest end of an answered request's content that
        // leaves the budget after it, or the latest end.
        let floor = before
            .iter()
            .rposition(|block| {
                matches!(block, Block::CompactionBoundary(_) | Block::CompactionMarker(_))
            })
            .map_or(0, |at| at + 1);
        // Each answered request's end, with whether the pass's model
        // answered it.
        let mut ends: Vec<(usize, bool)> = Vec::new();
        for (at, block) in before.iter().enumerate().skip(floor) {
            let Block::Response(response) = block else {
                continue;
            };
            let current = response.model.provider_id == boundary.model.provider_id
                && response.model.model.id == boundary.model.model.id;
            let mut answer = at;
            while answer > floor
                && matches!(
                    before[answer - 1],
                    Block::Thinking(_) | Block::RedactedThinking(_) | Block::Text(_) | Block::ToolCall(_)
                )
            {
                answer -= 1;
            }
            if ends.last().map(|(end, _)| *end) != Some(answer) {
                ends.push((answer, current));
            }
        }
        // The latest end of a request the pass's model answered that leaves
        // the budget after it, or the latest such end; without one, the
        // latest end of any.
        let current: Vec<usize> = ends.iter().filter(|(_, current)| *current).map(|(end, _)| *end).collect();
        let expected = match current.last() {
            Some(&latest) => Some(
                current
                    .iter()
                    .rev()
                    .copied()
                    .find(|end| weigh(*end) >= budget)
                    .unwrap_or(latest),
            ),
            None => ends.last().map(|(end, _)| *end),
        };
        if let Some(expected) = expected {
            assert_eq!(index, expected, "{context}: the cut of {:?} (ends {ends:?})", boundary.summary);
        }
        // The user's words: within their budget, each as the user wrote it
        // but for the oldest, which may be cut to its text.
        let kept = &boundary.kept;
        let words: u64 = kept.iter().map(|kept| content_tokens(&kept.content, &request)).sum();
        assert!(words <= words_budget, "{context}: {words} tokens of kept words");
        let written: Vec<Vec<UserContentBlock>> = before[start..index]
            .iter()
            .flat_map(|block| match block {
                Block::User(user) => vec![user.content.clone()],
                Block::Steer(steer) => vec![steer.content.clone()],
                Block::CompactionBoundary(previous) => {
                    previous.kept.iter().map(|kept| kept.content.clone()).collect()
                }
                _ => Vec::new(),
            })
            .collect();
        for (at, KeptMessage { content }) in kept.iter().enumerate() {
            let whole = written.contains(content);
            let cut = at == 0
                && content.iter().all(|part| matches!(part, UserContentBlock::Text { .. }))
                && written.iter().any(|message| {
                    let texts: Vec<&str> = message
                        .iter()
                        .filter_map(|part| match part {
                            UserContentBlock::Text { text } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect();
                    let UserContentBlock::Text { text } = &content[0] else {
                        return false;
                    };
                    texts.first().is_some_and(|first| first.starts_with(text.as_str()))
                });
            assert!(whole || cut, "{context}: kept {content:?} is no message of the user's");
        }
        // The estimate after the pass is under the threshold, or the pass
        // freed nothing.
        let after_blocks = &blocks[index..=marker];
        let after_view = self.model_view(index, after_blocks);
        let after = context_tokens(&self.view(&after_view, model), SYSTEM_PROMPT);
        let before_tokens = context_tokens(&request, SYSTEM_PROMPT);
        let threshold = token_threshold(window, model.output_limit);
        assert!(
            after < threshold || after >= before_tokens,
            "{context}: {before_tokens} tokens before the pass, {after} after, threshold {threshold}"
        );
    }
}

/// How a summary request's history ended: with the user's input, which
/// the instruction joins, or with a tool's results or an answer, which it
/// follows as a message of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum HistoryEnd {
    Input,
    Results,
    Answer,
}

/// Every summary request is the session's next request with the instruction
/// added.
///
/// Every builder encodes it as it encodes that request, but for the
/// instruction. It begins with the latest request the session sent to the same
/// model since the summary request before, with the same system prompt and
/// tools. Returns how its history ended.
fn check_summary(earlier: &[InferenceRequest], summary: &InferenceRequest, context: &str) -> HistoryEnd {
    let items = without_instruction(summary).unwrap_or_else(|| {
        panic!("{context}: the instruction is not the last block of the last user message")
    });
    let end = match items.last() {
        Some(InferenceItem::UserMessage { .. } | InferenceItem::UserSteer { .. })
            if items.len() == summary.items.len() =>
        {
            HistoryEnd::Input
        }
        Some(InferenceItem::ToolResult { .. }) => HistoryEnd::Results,
        _ => HistoryEnd::Answer,
    };
    // The next request: the same history and the blocks that give it, with
    // the entries a vendor kept on them, without the instruction.
    let next = InferenceRequest {
        items: items.clone().into(),
        blocks: summary
            .blocks
            .iter()
            .filter(|block| block.items.end <= items.len())
            .cloned()
            .collect(),
        ..summary.clone()
    };
    for builder in BUILDERS {
        assert_eq!(
            without_instruction_json(
                uncached(builder.json(summary)),
                matches!(builder, Builder::ChatCompletions | Builder::Grok)
            ),
            Some(uncached(builder.json(&next))),
            "{context}: {builder:?}: the summary request encodes the history otherwise"
        );
    }
    let previous = earlier.iter().rposition(is_summary).map_or(0, |index| index + 1);
    if let Some(sent) = earlier[previous..]
        .iter()
        .rev()
        .find(|request| request.model_id == summary.model_id)
    {
        assert!(
            items.starts_with(&sent.items),
            "{context}: the summary request does not begin with the latest request: {items:?}"
        );
        assert_eq!(
            (&summary.system_prompt, &summary.tools),
            (&sent.system_prompt, &sent.tools),
            "{context}"
        );
    }
    end
}

/// An encoded request without the summary instruction.
///
/// Each value that holds it goes, with the object or the array element it is
/// in, and an array that held nothing else goes with it, up to the message.
///
/// A format that joins a message's texts into one string, as Chat Completions
/// and Grok do, keeps the input's text before a line break and the instruction:
/// the text stays a prefix of what its vendor caches, which caches prefixes
/// without marks. `joins` says whether `builder` is such a format.
fn without_instruction_json(value: Value, joins: bool) -> Option<Value> {
    match value {
        Value::String(text) if text == COMPACTION_SUMMARY_INSTRUCTION => None,
        Value::String(text)
            if joins && text.ends_with(&format!("\n{COMPACTION_SUMMARY_INSTRUCTION}")) =>
        {
            let kept = text.len() - COMPACTION_SUMMARY_INSTRUCTION.len() - 1;
            Some(Value::String(text[..kept].to_owned()))
        }
        Value::Object(fields) => {
            let mut kept = serde_json::Map::new();
            for (key, field) in fields {
                kept.insert(key, without_instruction_json(field, joins)?);
            }
            Some(Value::Object(kept))
        }
        Value::Array(elements) => {
            let held = !elements.is_empty();
            let kept: Vec<Value> = elements
                .into_iter()
                .filter_map(|element| without_instruction_json(element, joins))
                .collect();
            (!kept.is_empty() || !held).then_some(Value::Array(kept))
        }
        other => Some(other),
    }
}

/// `body` without the Anthropic cache marks, which the vendor does not count
/// as content.
fn uncached(mut body: Value) -> Value {
    fn strip(value: &mut Value) {
        match value {
            Value::Object(fields) => {
                fields.remove("cache_control");
                fields.values_mut().for_each(strip);
            }
            Value::Array(items) => items.iter_mut().for_each(strip),
            _ => {}
        }
    }
    strip(&mut body);
    body
}

/// The signatures of `request`'s reasoning that `tag` marks as its vendor's,
/// without the tag.
///
/// With `checks_history`, a vendor that checks reasoning against the history
/// before it, not the reasoning kept past a summary.
fn signatures(request: &InferenceRequest, tag: &str, checks_history: bool) -> Vec<String> {
    request
        .items
        .iter()
        .filter_map(|item| match item {
            InferenceItem::AssistantThinking {
                signature: Some(signature),
                kept_past_summary,
                ..
            } if !(checks_history && *kept_past_summary) => {
                signature.strip_prefix(tag).map(str::to_owned)
            }
            _ => None,
        })
        .collect()
}

/// Whether `builder`'s encoding of `request` is one its vendor takes.
pub(super) fn valid(builder: Builder, request: &InferenceRequest) -> Result<(), String> {
    let body = builder.json(request);
    match builder {
        Builder::Anthropic => messages_valid(&body["messages"], &signatures(request, "anthropic:", true)),
        Builder::ClaudeCode => entries_valid(&body, request),
        Builder::Responses => input_valid(&body["input"], &signatures(request, "openai:", false)),
        Builder::Codex => input_valid(&body["input"], &signatures(request, "codex:", false)),
        Builder::ChatCompletions | Builder::Grok => chat_valid(&body["messages"]),
        Builder::Google => contents_valid(&body["contents"], &signatures(request, "google:", false)),
    }
}

fn array(value: &Value) -> Result<&Vec<Value>, String> {
    value.as_array().ok_or_else(|| format!("not an array: {value}"))
}

/// Messages API: user first and last, roles alternating; each assistant
/// message's calls answered in the next message, and a result only for a
/// call of the message before it; signed reasoning only Anthropic's.
fn messages_valid(messages: &Value, signed: &[String]) -> Result<(), String> {
    let messages = array(messages)?;
    let roles: Vec<&str> = messages.iter().map(|m| m["role"].as_str().unwrap_or("")).collect();
    if roles.first() != Some(&"user") || roles.last() != Some(&"user") {
        return Err(format!("roles {roles:?}"));
    }
    if roles.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(format!("roles do not alternate: {roles:?}"));
    }
    let mut calls: Vec<String> = Vec::new();
    for message in messages {
        let content = array(&message["content"])?;
        if message["role"] == "assistant" {
            if !calls.is_empty() {
                return Err(format!("calls without results: {calls:?}"));
            }
            for block in content {
                match block["type"].as_str() {
                    Some("tool_use") => calls.push(block["id"].as_str().unwrap_or("").to_owned()),
                    Some("thinking") => {
                        let signature = block["signature"].as_str().unwrap_or("");
                        if !signed.iter().any(|known| known == signature) {
                            return Err(format!("reasoning signed {signature:?}"));
                        }
                    }
                    Some("redacted_thinking") => return Err("redacted reasoning".into()),
                    _ => {}
                }
            }
        } else {
            let results: Vec<String> = content
                .iter()
                .filter(|block| block["type"] == "tool_result")
                .map(|block| block["tool_use_id"].as_str().unwrap_or("").to_owned())
                .collect();
            let mut expected = calls.clone();
            expected.sort();
            let mut answered = results.clone();
            answered.sort();
            if expected != answered {
                return Err(format!("results {results:?} answer calls {calls:?}"));
            }
            calls.clear();
        }
    }
    Ok(())
}

/// Claude Code's session: an entry for each user message, every result
/// after its call, no call without a result, and no reasoning, which only
/// the CLI's own entries carry.
fn entries_valid(entries: &Value, request: &InferenceRequest) -> Result<(), String> {
    let messages = request
        .items
        .iter()
        .filter(|item| {
            matches!(item, InferenceItem::UserMessage { .. } | InferenceItem::UserSteer { .. })
        })
        .count();
    let written = array(entries)?
        .iter()
        .filter(|entry| {
            entry["type"] == "user"
                && !entry["message"]["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .any(|block| block["type"] == "tool_result")
        })
        .count();
    if written != messages {
        return Err(format!("{written} user entries for {messages} user messages"));
    }
    let mut calls: Vec<String> = Vec::new();
    for entry in array(entries)? {
        for block in entry["message"]["content"].as_array().into_iter().flatten() {
            match block["type"].as_str() {
                Some("tool_use") => calls.push(block["id"].as_str().unwrap_or("").to_owned()),
                Some("tool_result") => {
                    let id = block["tool_use_id"].as_str().unwrap_or("");
                    let Some(at) = calls.iter().position(|call| call == id) else {
                        return Err(format!("a result of {id} without its call"));
                    };
                    calls.remove(at);
                }
                Some("thinking" | "redacted_thinking") => {
                    return Err("reasoning no entry of the CLI's stands for".into());
                }
                _ => {}
            }
        }
    }
    if calls.is_empty() {
        Ok(())
    } else {
        Err(format!("calls without results: {calls:?}"))
    }
}

/// Responses input: a user message first, every output after its call, no
/// call without its output, reasoning only of the provider's own vendor.
fn input_valid(input: &Value, signed: &[String]) -> Result<(), String> {
    let input = array(input)?;
    if input.first().map(|item| &item["role"]) != Some(&Value::from("user")) {
        return Err("the input does not start with the user".into());
    }
    let mut calls: Vec<String> = Vec::new();
    for item in input {
        match item["type"].as_str() {
            Some("function_call") => calls.push(item["call_id"].as_str().unwrap_or("").to_owned()),
            Some("function_call_output") => {
                let id = item["call_id"].as_str().unwrap_or("");
                let Some(at) = calls.iter().position(|call| call == id) else {
                    return Err(format!("an output of {id} without its call"));
                };
                calls.remove(at);
            }
            Some("reasoning") => {
                let id = item["id"].as_str().unwrap_or("");
                if !signed.iter().any(|signature| signature.contains(&format!("\"{id}\""))) {
                    return Err(format!("reasoning {id} of another vendor"));
                }
            }
            _ => {}
        }
    }
    if calls.is_empty() {
        Ok(())
    } else {
        Err(format!("calls without outputs: {calls:?}"))
    }
}

/// Chat Completions: the system prompt, then the user; each assistant
/// message's calls answered by the tool messages right after it, and a tool
/// message only for such a call.
fn chat_valid(messages: &Value) -> Result<(), String> {
    let messages = array(messages)?;
    let roles: Vec<&str> = messages.iter().map(|m| m["role"].as_str().unwrap_or("")).collect();
    if roles.first() != Some(&"system") || roles.get(1) != Some(&"user") {
        return Err(format!("roles {roles:?}"));
    }
    let mut calls: Vec<String> = Vec::new();
    for message in messages {
        match message["role"].as_str() {
            Some("tool") => {
                let id = message["tool_call_id"].as_str().unwrap_or("");
                let Some(at) = calls.iter().position(|call| call == id) else {
                    return Err(format!("a tool message for {id} without its call"));
                };
                calls.remove(at);
            }
            role => {
                if !calls.is_empty() {
                    return Err(format!("{role:?} before the results of {calls:?}"));
                }
                if role == Some("assistant") {
                    calls = message["tool_calls"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|call| call["id"].as_str().unwrap_or("").to_owned())
                        .collect();
                }
            }
        }
    }
    if calls.is_empty() {
        Ok(())
    } else {
        Err(format!("calls without results: {calls:?}"))
    }
}

/// Gemini contents: user first and last, roles alternating; each model
/// content's calls answered by as many responses in the next content;
/// thought signatures only Google's.
fn contents_valid(contents: &Value, signed: &[String]) -> Result<(), String> {
    let contents = array(contents)?;
    let roles: Vec<&str> = contents.iter().map(|c| c["role"].as_str().unwrap_or("")).collect();
    if roles.first() != Some(&"user") || roles.last() != Some(&"user") {
        return Err(format!("roles {roles:?}"));
    }
    if roles.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(format!("roles do not alternate: {roles:?}"));
    }
    let mut calls: Vec<String> = Vec::new();
    for content in contents {
        let parts = array(&content["parts"])?;
        let responses: Vec<String> = parts
            .iter()
            .filter_map(|part| part["functionResponse"]["name"].as_str().map(str::to_owned))
            .collect();
        if content["role"] == "model" {
            calls = parts
                .iter()
                .filter_map(|part| part["functionCall"]["name"].as_str().map(str::to_owned))
                .collect();
            for part in parts {
                if let Some(signature) = part["thoughtSignature"].as_str()
                    && !signed.iter().any(|known| known == signature)
                {
                    return Err(format!("a thought signed {signature:?}"));
                }
            }
        } else {
            if responses != calls {
                return Err(format!("responses {responses:?} answer calls {calls:?}"));
            }
            calls.clear();
        }
    }
    Ok(())
}

/// Drives `seed`'s session and checks it after every step.
/// Drives the session of `seed`; returns how the histories of its summary
/// requests ended.
async fn drive(seed: u64) -> HashSet<HistoryEnd> {
    let mut driver = Driver::new(seed).await;
    for step in 0..STEPS {
        driver.step().await;
        driver.check(step).await;
    }
    driver.shared.session.replace(None);
    let _ = driver.session.dispose().await;
    driver.history_ends
}

// Four seeds of 24 steps take every kind of step, about 400 requests and 120
// passes, each request encoded by seven builders and checked: about 5 s in a
// debug build.
//
// It is the one test that meets passes at moments no scripted scenario reaches,
// such as right after a Stop, a restart or a switch.
#[tokio::test(flavor = "local", start_paused = true)]
async fn generated_sessions_stay_valid_for_every_provider_through_every_pass() {
    let mut history_ends = HashSet::new();
    for seed in SEEDS.chain(FIXED_SEEDS.iter().copied()) {
        history_ends.extend(drive(seed).await);
    }
    // The summary requests met both ends of a history that the instruction
    // meets inside a user message: the user's input, which it joins, and a
    // tool's results.
    assert!(
        history_ends.contains(&HistoryEnd::Input) && history_ends.contains(&HistoryEnd::Results),
        "{history_ends:?}"
    );
}
