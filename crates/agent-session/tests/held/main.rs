//! What a live session holds of a long conversation, measured with a counting
//! allocator, in its own binary.
//!
//! The allocator counts every allocation of the process, so a test running next
//! to it would be counted as the session's.
//!
//! One test: a 20,000-block path with a compaction boundary near its end,
//! written to a memory store, then restored. It prints what the whole path
//! weighs decoded, which the session held before it held only its window, and
//! what the restored session holds; about a second in a debug build.

use std::{
    alloc::{GlobalAlloc, Layout, System},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicIsize, Ordering},
    },
};

use demi_agent_session::{
    AgentSession, NewContext, SeenContext, SessionConfig, SessionDeps, SessionRuntime, StepOutcomes,
    SystemPrompt, ToolInvocation,
};
use demi_agent_store::{
    AgentTreeStore, CheckpointState, CheckpointUpdate, NodeRecord,
    testing::{MemoryTreeStore, test_model},
};
use demi_agent_transcript::{ForkEnd, testing::SequentialIds};
use demi_provider_common::{
    ToolDefinition,
    testing::{FixedClock, ScriptedRuntime},
};
use demi_shared_gates::{ActivityGate, GateLease, Purpose, Reservation};
use demi_shared_types::{Block, ModelSelection, NodeId, SessionPhase, Timestamp, TurnId};
use futures_util::future::LocalBoxFuture;
use serde_json::json;

/// The system allocator, counting the bytes the process holds.
struct Counting;

static HELD: AtomicIsize = AtomicIsize::new(0);
/// The most `HELD` reached since the last [`peak_from_now`].
static PEAK: AtomicIsize = AtomicIsize::new(0);

fn grew(by: isize) {
    let now = HELD.fetch_add(by, Ordering::Relaxed) + by;
    PEAK.fetch_max(now, Ordering::Relaxed);
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        grew(layout.size() as isize);
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        HELD.fetch_sub(layout.size() as isize, Ordering::Relaxed);
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        grew(new_size as isize - layout.size() as isize);
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

fn held() -> isize {
    HELD.load(Ordering::Relaxed)
}

/// Starts a new peak at what the process holds now, and answers it.
fn peak_from_now() -> isize {
    let now = held();
    PEAK.store(now, Ordering::Relaxed);
    now
}

/// A runtime with no tools, which the restore needs and nothing calls.
struct Idle(ActivityGate);

impl SessionRuntime for Idle {
    fn enter_action(&self) -> LocalBoxFuture<'_, GateLease> {
        Box::pin(self.0.enter(Purpose::Demand))
    }

    fn reserve_edit(&self) -> LocalBoxFuture<'_, Result<Option<Reservation>, String>> {
        Box::pin(async { Ok(None) })
    }

    fn system_prompt<'a>(
        &'a self,
        _model: &'a ModelSelection,
    ) -> LocalBoxFuture<'a, Result<SystemPrompt, String>> {
        Box::pin(async { Ok(SystemPrompt::identity("You are a test.")) })
    }

    fn context_window<'a>(&'a self, model: &'a ModelSelection) -> LocalBoxFuture<'a, u32> {
        Box::pin(async move { model.model.context_window })
    }

    fn preamble(&self) -> LocalBoxFuture<'_, Option<String>> {
        Box::pin(async { None })
    }

    fn context<'a>(
        &'a self,
        _seen: &'a [SeenContext<'a>],
        _turn: &'a TurnId,
    ) -> LocalBoxFuture<'a, Vec<NewContext>> {
        Box::pin(async { Vec::new() })
    }

    fn tools(&self) -> Arc<[ToolDefinition]> {
        Arc::from(Vec::new())
    }

    fn agent(&self) -> Option<u64> {
        Some(0)
    }

    fn invoke_step(&self, _calls: Vec<ToolInvocation>) -> StepOutcomes<'_> {
        unreachable!("the measured session runs no turn")
    }
}

/// A vendor record of a block, as Claude Code keeps one: about 1.5 KB.
fn entry(n: usize, kind: &str) -> serde_json::Value {
    json!({
        "type": "assistant",
        "uuid": format!("5b0c7d5e-{n:08}-4b0e-9a62-6f3a1c2d4e5f"),
        "parentUuid": format!("5b0c7d5e-{:08}-4b0e-9a62-6f3a1c2d4e5f", n.saturating_sub(1)),
        "timestamp": "2026-10-11T12:00:00.000Z",
        "isSidechain": false,
        "sessionId": "4813494d-137e-4631-bba3-01d5acab6e7b",
        "message": {
            "id": format!("msg_{n}"),
            "type": "message",
            "role": "assistant",
            "model": "claude-opus-4-5",
            "content": [{"type": kind, "text": "x".repeat(900)}],
            "usage": {"input_tokens": 3, "output_tokens": 120, "cache_read_input_tokens": 90000},
        },
    })
}

/// Block `n` of the fixture: turns of a message, a thinking, a text, a
/// shell call with its output and the response, and at `boundary` a
/// compaction boundary.
fn block(n: usize, boundary: usize) -> Block {
    let model = serde_json::to_value(test_model()).unwrap();
    let id = format!("b{n}");
    let value = if n == boundary {
        json!({"type": "compaction_boundary", "id": id, "createdAt": "2026-10-11T12:00:00Z",
            "model": model, "summary": "s".repeat(4000), "summaryTokens": 1000, "kept": []})
    } else {
        match n % 5 {
            0 => json!({"type": "user", "id": id, "createdAt": "2026-10-11T12:00:00Z", "model": model,
                "turnId": format!("turn-{n}"), "content": [{"type": "text", "text": "u".repeat(300)}], "preamble": null}),
            1 => json!({"type": "thinking", "id": id, "createdAt": "2026-10-11T12:00:00Z", "model": model,
                "text": "t".repeat(600), "signature": "EqoBCkgIARABGAIiQL", "entries": [entry(n, "thinking")]}),
            2 => json!({"type": "text", "id": id, "createdAt": "2026-10-11T12:00:00Z", "model": model,
                "text": "a".repeat(600), "forkable": true, "entries": [entry(n, "text")]}),
            3 => json!({"type": "tool_call", "id": id, "createdAt": "2026-10-11T12:00:00Z", "model": model,
                "toolUseId": format!("toolu_{n}"), "toolName": "shell", "input": "{\"script\":\"cargo test\"}",
                "status": "completed", "output": [{"type": "text", "text": "o".repeat(2000)}],
                "view": null, "endedAt": null, "entries": [entry(n, "tool_use")]}),
            _ => json!({"type": "response", "id": id, "createdAt": "2026-10-11T12:00:00Z", "model": model,
                "usage": {"inputTokens": 3, "outputTokens": 120, "cacheReadTokens": 90000, "cacheWriteTokens": 0}}),
        }
    };
    serde_json::from_value(value).expect("a fixture block decodes")
}

fn root() -> NodeId {
    NodeId::try_from("root").unwrap()
}

#[tokio::test(flavor = "local")]
async fn a_restored_session_holds_only_the_window_after_its_last_boundary() {
    const BLOCKS: usize = 20_000;
    const BOUNDARY: usize = 19_800;
    let path: Vec<Block> = (0..BLOCKS).map(|n| block(n, BOUNDARY)).collect();
    let json_bytes = serde_json::to_vec(&path).unwrap().len();
    let store = MemoryTreeStore::new();
    store
        .create_node(
            NodeRecord::root(root(), Timestamp::UNIX_EPOCH),
            CheckpointUpdate {
                state: CheckpointState {
                    phase: SessionPhase::Idle,
                    queue: Vec::new(),
                    agent_inputs: Vec::new(),
                    reports: Vec::new(),
                    next_turn_reports: Vec::new(),
                    intervals: Vec::new(),
                    model: test_model(),
                    edits: Vec::new(),
                    account: None,
                },
                block_count: path.len(),
                changed_blocks: path.into_iter().enumerate().collect(),
                moved_from: Some(0),
                version: false,
            },
        )
        .await
        .unwrap();

    // What the whole path weighs decoded: a session held it all before.
    let before = held();
    let whole = store.path(&root());
    let path_bytes = held() - before;
    drop(whole);

    let before = held();
    let deps = SessionDeps {
        runtime: Rc::new(Idle(ActivityGate::new())),
        store: store.session_store(&root()),
        ids: Rc::new(SequentialIds::new("held")),
        clock: Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
        config: SessionConfig::default(),
    };
    let checkpoint = store.checkpoint(&root()).unwrap();
    let (session, _) = AgentSession::restore(
        checkpoint,
        root(),
        Box::new(ScriptedRuntime::new([])),
        deps,
        &[],
    )
    .await
    .unwrap();
    let session_bytes = held() - before;
    let window = session.transcript();
    eprintln!(
        "path: {BLOCKS} blocks, {json_bytes} bytes of JSON, {path_bytes} bytes decoded; \
         session: blocks {}.., {} held, {session_bytes} bytes",
        window.start,
        window.blocks.len(),
    );
    assert_eq!((window.start, window.blocks.len()), (BOUNDARY, BLOCKS - BOUNDARY));
    // The window is a hundredth of the path; the session holds about that
    // and its own state, never the path.
    assert!(session_bytes < path_bytes / 20, "{session_bytes} of {path_bytes}");
    drop(window);

    // A Fork of it names the rows before the window and copies only the
    // window's: at no moment does it hold the path.
    let before = peak_from_now();
    let seed = session.prepare_fork(ForkEnd::Newest).await.unwrap();
    let seed_peak = PEAK.load(Ordering::Relaxed) - before;
    let fork = NodeId::try_from("fork").unwrap();
    let record = NodeRecord { parent: Some(root()), number: 1, ..NodeRecord::root(fork.clone(), Timestamp::UNIX_EPOCH) };
    // The memory store is the database here: its copy is its rows' index,
    // which a database keeps on disk.
    let before = peak_from_now();
    seed.create(&*store, record, None).await.unwrap();
    let copy_peak = PEAK.load(Ordering::Relaxed) - before;
    drop(seed);
    eprintln!("fork: the seed {seed_peak} bytes at its peak; the memory store's copy {copy_peak}");
    assert_eq!(store.session_store(&fork).load().await.unwrap().unwrap().block_count(), BLOCKS - 2);
    assert!(seed_peak < path_bytes / 20, "{seed_peak} of {path_bytes}");
    let _ = session.dispose().await;
}
