//! What a session reads of the blocks before its window.
//!
//! A page, a switch, a Fork and a turn's end each take what they need from the
//! store, and none copies the path before the window into the session. Each
//! test writes a path into a memory store and restores the session from it.

use demi_agent_session::ForkSeed;
use demi_agent_store::{CheckpointState, CheckpointUpdate, testing::test_model};
use demi_agent_transcript::ForkEnd;

use super::*;

/// The block of type `kind` named `id` with `fields`, the model the tests
/// use, written as a store keeps it.
fn block(kind: &str, id: &str, fields: serde_json::Value) -> Block {
    let mut value = json!({ "type": kind, "id": id, "createdAt": "2026-10-11T12:00:00Z", "model": test_model() });
    value.as_object_mut().unwrap().extend(fields.as_object().unwrap().clone());
    serde_json::from_value(value).expect("a test block decodes")
}

fn user(id: &str, turn: &str, text: &str) -> Block {
    block("user", id, json!({ "turnId": turn, "content": [{ "type": "text", "text": text }], "preamble": null }))
}

fn said(id: &str, text: &str) -> Block {
    block("text", id, json!({ "text": text, "forkable": true }))
}

fn response(id: &str) -> Block {
    block("response", id, json!({ "usage": { "inputTokens": 1, "outputTokens": 1, "cacheReadTokens": 0, "cacheWriteTokens": 0 } }))
}

fn boundary(id: &str) -> Block {
    block("compaction_boundary", id, json!({ "summary": "summary", "summaryTokens": 10, "kept": [] }))
}

fn idle() -> CheckpointState {
    CheckpointState {
        phase: SessionPhase::Idle,
        queue: Vec::new(),
        agent_inputs: Vec::new(),
        reports: Vec::new(),
        next_turn_reports: Vec::new(),
        watches: Vec::new(),
        model: test_model(),
        edits: Vec::new(),
        account: None,
        turn_speed: None,
    }
}

/// `path` as the root's in a new memory store.
async fn stored(path: &[Block]) -> Rc<MemoryTreeStore> {
    let store = MemoryTreeStore::new();
    store
        .create_node(
            NodeRecord::root(root(), Timestamp::UNIX_EPOCH),
            CheckpointUpdate {
                state: idle(),
                changed_blocks: path.iter().cloned().enumerate().collect(),
                block_count: path.len(),
                moved_from: Some(0),
                version: false,
            },
        )
        .await
        .unwrap();
    store
}

async fn restored(store: &Rc<MemoryTreeStore>) -> AgentSession {
    restore_configured(
        store.checkpoint(&root()).unwrap(),
        store,
        &ScriptedRuntime::new([]),
        test_runtime(Vec::new()),
        Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
        SessionConfig::default(),
    )
    .await
    .0
}

/// Request A at 0 whose tail runs into the window that its compaction
/// started at 4, then B at 6 and C at 9.
fn across_a_boundary(tail: &str) -> Vec<Block> {
    vec![
        user("a", "ta", "A"),
        said("a1", "A's work"),
        response("a2"),
        said("a3", "more of A"),
        boundary("b"),
        said("a4", tail),
        user("u-b", "tb", "B"),
        said("b1", "B's answer"),
        response("b2"),
        user("u-c", "tc", "C"),
        said("c1", "C's answer"),
        response("c2"),
    ]
}

// The latest page of a window reaches no block before the window: a reset with
// nothing held starts at the window when it holds no more than a page, and a
// page before its end when it holds more.
//
// Two restored sessions, a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_reset_of_a_window_is_at_most_a_page_and_starts_within_the_window() {
    let session = restored(&stored(&across_a_boundary("the rest of A")).await).await;
    assert_eq!(session.transcript().start, 4);
    let reset = session.reset(None, None);
    assert_eq!((reset.start, reset.length), (4, 12));

    let mut path = vec![user("a", "ta", "A"), boundary("b")];
    path.extend((0..1_500).map(|n| said(&format!("s{n}"), "words")));
    let session = restored(&stored(&path).await).await;
    assert_eq!(session.transcript().start, 1);
    let reset = session.reset(None, None);
    assert_eq!(
        (reset.start, reset.length, reset.blocks.len()),
        (502, 1_502, demi_conversation_socket_protocol::PAGE_BLOCKS)
    );
}

// A Fork of a session whose window starts at 4 keeps the blocks before it as
// the store's rows: the seed names them and holds only the window's, and the
// node it starts holds the path through the text.
//
// One store, a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_forks_seed_names_the_rows_before_the_window_and_its_node_holds_them() {
    let path = across_a_boundary("the rest of A");
    let store = stored(&path).await;
    let session = restored(&store).await;
    let seed: ForkSeed = session.prepare_fork(ForkEnd::At(&BlockId::try_from("b1").unwrap())).await.unwrap();
    assert_eq!(seed.prefix.as_ref().map(|prefix| prefix.length), Some(4));
    assert_eq!((seed.checkpoint.start, seed.checkpoint.transcript.clone()), (4, path[4..8].to_vec()));

    let fork = NodeId::try_from("fork").unwrap();
    let record = NodeRecord { parent: Some(root()), number: 1, ..NodeRecord::root(fork.clone(), Timestamp::UNIX_EPOCH) };
    seed.create(&*store, record, None).await.unwrap();
    assert_eq!(store.path(&fork), path[..8]);

    // A text before the window ends a Fork too: its rows are all named.
    let seed = session.prepare_fork(ForkEnd::At(&BlockId::try_from("a1").unwrap())).await.unwrap();
    assert_eq!(seed.prefix.as_ref().map(|prefix| prefix.length), Some(2));
    assert!(seed.checkpoint.transcript.is_empty());
}

// The end of a turn whose message is before the window is read from that
// message through the next one, never from the window's start: A's own
// answer, not B's. One restored session, a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_turn_before_the_window_is_read_through_its_own_end() {
    let path = vec![
        user("a", "ta", "A"),
        said("a1", "A's answer"),
        response("a2"),
        user("u-b", "tb", "B"),
        said("b1", "B's answer"),
        boundary("b"),
        said("b2", "B's last answer"),
        response("b3"),
    ];
    let session = restored(&stored(&path).await).await;
    assert_eq!(session.transcript().start, 5);
    let blocks = session.turn_blocks(&turn("ta")).await.unwrap().unwrap();
    assert_eq!(blocks, path[1..4]);
    // A turn the window's boundary cut through goes on into the window.
    let blocks = session.turn_blocks(&turn("tb")).await.unwrap().unwrap();
    assert_eq!(blocks, [path[4].clone(), path[6].clone(), path[7].clone()]);
    assert_eq!(session.turn_blocks(&turn("tz")).await.unwrap(), None);
}

/// A node's store whose reads fail once a switch committed, as a database
/// that goes away right after it: what a switch adopts must come from the
/// switch itself.
struct LostAfterSwitch {
    inner: Rc<dyn SessionStore>,
    switched: Cell<bool>,
}

impl LostAfterSwitch {
    fn gone<'a, T: 'a>(&self, read: LocalBoxFuture<'a, Result<T, StoreError>>) -> LocalBoxFuture<'a, Result<T, StoreError>> {
        if self.switched.get() {
            return Box::pin(async { Err(StoreError::Failed("the database is gone".to_owned())) });
        }
        read
    }
}

impl SessionStore for LostAfterSwitch {
    fn save(&self, update: CheckpointUpdate) -> LocalBoxFuture<'_, Result<(), StoreError>> {
        self.inner.save(update)
    }

    fn load(&self) -> LocalBoxFuture<'_, Result<Option<Checkpoint>, StoreError>> {
        self.gone(self.inner.load())
    }

    fn blocks(&self, range: std::ops::Range<usize>) -> LocalBoxFuture<'_, Result<demi_agent_store::Rows, StoreError>> {
        self.gone(self.inner.blocks(range))
    }

    fn position<'a>(&'a self, id: &'a BlockId) -> LocalBoxFuture<'a, Result<Option<usize>, StoreError>> {
        self.gone(self.inner.position(id))
    }

    fn last_of(&self, kind: demi_agent_store::Kind, end: usize) -> LocalBoxFuture<'_, Result<Option<usize>, StoreError>> {
        self.gone(self.inner.last_of(kind, end))
    }

    fn next_of(&self, kind: demi_agent_store::Kind, from: usize) -> LocalBoxFuture<'_, Result<Option<usize>, StoreError>> {
        self.gone(self.inner.next_of(kind, from))
    }

    fn blocks_of(
        &self,
        kinds: Vec<demi_agent_store::Kind>,
        range: std::ops::Range<usize>,
    ) -> LocalBoxFuture<'_, Result<Vec<(usize, Block)>, StoreError>> {
        self.gone(self.inner.blocks_of(kinds, range))
    }

    fn turn_position<'a>(&'a self, turn: &'a TurnId) -> LocalBoxFuture<'a, Result<Option<usize>, StoreError>> {
        self.gone(self.inner.turn_position(turn))
    }

    fn earlier(&self, end: usize) -> LocalBoxFuture<'_, Result<demi_agent_store::EarlierRows, StoreError>> {
        self.gone(self.inner.earlier(end))
    }

    fn prefix(&self, length: usize) -> LocalBoxFuture<'_, Result<Option<demi_agent_store::StoredPrefix>, StoreError>> {
        self.gone(self.inner.prefix(length))
    }

    fn switch(
        &self,
        position: usize,
        version: u32,
        held_from: usize,
    ) -> LocalBoxFuture<'_, Result<demi_agent_store::Switched, StoreError>> {
        Box::pin(async move {
            let switched = self.inner.switch(position, version, held_from).await?;
            self.switched.set(true);
            Ok(switched)
        })
    }

    fn blobs(&self) -> &dyn demi_agent_store::media::BlobStore {
        self.inner.blobs()
    }
}

// An edited message that a compaction carried past its boundary has its
// versions where it was: switching back to the first version changes the path
// from before the window, and the session takes what it then holds from the
// switch's own transaction, so a store lost right after the commit fails
// nothing.
//
// One restored session, a few milliseconds.
#[tokio::test(flavor = "local")]
async fn a_switch_before_the_window_adopts_what_its_commit_read() {
    let first = vec![user("u1", "t1", "one"), said("a1", "first"), user("u2", "t2", "two"), said("a2", "second")];
    let store = stored(&first).await;
    let node = store.session_store(&root());
    let rewrite = |from: usize, blocks: Vec<Block>, version: bool| CheckpointUpdate {
        state: idle(),
        block_count: from + blocks.len(),
        changed_blocks: (from..).zip(blocks).collect(),
        moved_from: Some(from),
        version,
    };
    let edited = vec![user("u2e", "t2e", "two, edited"), said("a2e", "second, edited")];
    node.save(rewrite(2, edited.clone(), true)).await.unwrap();
    // The pass puts its boundary after a block of its own and carries the
    // edited message past it.
    let carried = [vec![said("x", "kept"), boundary("b")], edited].concat();
    node.save(rewrite(2, carried, false)).await.unwrap();

    let lost = Rc::new(LostAfterSwitch {
        inner: node,
        switched: Cell::new(false),
    });
    let deps = SessionDeps {
        runtime: Rc::new(test_runtime(Vec::new())),
        store: lost.clone(),
        ids: Rc::new(SequentialIds::new("restored")),
        clock: Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
        config: SessionConfig::default(),
    };
    let (session, _) = AgentSession::restore(
        store.checkpoint(&root()).unwrap(),
        root(),
        Box::new(ScriptedRuntime::new([])),
        deps,
        &[],
    )
    .await
    .unwrap();
    let held = session.transcript();
    assert_eq!(held.start, 3);

    session
        .switch_version(BlockId::try_from("u2e").unwrap(), 1, &held.version)
        .await
        .unwrap();
    assert!(lost.switched.get());
    assert_eq!(store.path(&root()), first);
    let held = session.transcript();
    assert_eq!((held.start, held.blocks), (0, first));
}
