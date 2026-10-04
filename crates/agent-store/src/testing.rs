//! Test support (feature `testing`): an in-memory tree store with the
//! contract's semantics and an in-memory blob namespace, the contract's cases
//! every realization passes, and the model selections, texts and images the
//! tests of the agent's crates build on.

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};

use demi_provider_common::UserPart;
use demi_shared_types::{
    B64Bytes, BlobRef, Block, CommandId, FileExtension, Model, ModelSelection, NodeId,
    QueuedMessage, Sequence, Timestamp, UserContentBlock,
};
use futures_util::future::LocalBoxFuture;

use crate::{
    AgentTreeStore, Checkpoint, CheckpointState, CheckpointUpdate, NodeClose, NodeRecord,
    SessionStore, StoreError, StoredOutput, media::BlobStore,
};

/// A model selection of the provider `provider` with a 100,000-token window.
pub fn model_of(provider: &str, model: &str) -> ModelSelection {
    ModelSelection {
        provider_id: provider.to_owned(),
        model: Model {
            id: model.to_owned(),
            name: model.to_owned(),
            context_window: 100_000,
            output_limit: None,
            thinking: Vec::new(),
            accepted_extensions: Some(Vec::new()),
        },
        thinking: None,
        service_tier_id: None,
    }
}

/// As [`model_of`], for a model that reads `extensions` natively.
pub fn model_reading(provider: &str, model: &str, extensions: &[FileExtension]) -> ModelSelection {
    let mut selection = model_of(provider, model);
    selection.model.accepted_extensions = Some(extensions.to_vec());
    selection
}

/// A PNG of `width` × `height` px whose pixels follow `seed`, for tests that
/// send images: a real one, since an image is decoded as it enters a
/// transcript (`runtime.md` § Images in the transcript).
pub fn png(width: u32, height: u32, seed: u8) -> B64Bytes {
    use image::{DynamicImage, Rgba, RgbaImage, codecs::png::PngEncoder};

    let pixels = RgbaImage::from_fn(width, height, |x, y| Rgba([x as u8, y as u8, seed, 255]));
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(pixels)
        .write_with_encoder(PngEncoder::new(&mut bytes))
        .expect("a PNG encodes into memory");
    B64Bytes::from(bytes)
}

/// The selection of the model `test-model` of the provider `stub`.
pub fn test_model() -> ModelSelection {
    model_of("stub", "test-model")
}

/// A message's content of one text.
pub fn text(text: &str) -> Vec<UserContentBlock> {
    vec![UserContentBlock::Text {
        text: text.to_owned(),
    }]
}

/// The same content as a request carries it.
pub fn sent_text(text: &str) -> Vec<UserPart> {
    vec![UserPart::Text(text.to_owned())]
}

/// One node as the store holds it.
#[derive(Debug, Clone)]
struct StoredNode {
    record: NodeRecord,
    state: CheckpointState,
    blocks: BTreeMap<usize, Block>,
    block_count: usize,
}

#[derive(Debug, Default, Clone)]
struct Stored {
    nodes: BTreeMap<NodeId, StoredNode>,
    saves: Vec<(NodeId, CheckpointUpdate)>,
    /// The next number of each sequence that gave one out.
    sequences: BTreeMap<Sequence, u64>,
    failing_saves: usize,
    save_hold: Option<Rc<Hold>>,
    number_hold: Option<Rc<Hold>>,
    /// The holds on reads of a node's children, by the node.
    children_holds: BTreeMap<NodeId, Rc<Hold>>,
    /// What the product's keeper stored of each ended command's output.
    outputs: BTreeMap<CommandId, StoredOutput>,
}

/// Calls waiting until a test lets them through.
#[derive(Debug, Default)]
struct Hold {
    waiting: Cell<usize>,
    open: Cell<bool>,
    released: tokio::sync::Notify,
}

impl Hold {
    /// Waits until the hold is released.
    async fn pass(&self) {
        self.waiting.set(self.waiting.get() + 1);
        while !self.open.get() {
            let released = self.released.notified();
            tokio::pin!(released);
            released.as_mut().enable();
            if self.open.get() {
                break;
            }
            released.await;
        }
        self.waiting.set(self.waiting.get() - 1);
    }
}

/// A hold on some of a [`MemoryTreeStore`]'s calls: each waits until
/// [`release`](Self::release), as a call behind a slow database would.
#[derive(Debug)]
pub struct StoreGate(Rc<Hold>);

impl StoreGate {
    /// How many calls wait.
    pub fn waiting(&self) -> usize {
        self.0.waiting.get()
    }

    /// Lets the waiting calls and every later one through.
    pub fn release(&self) {
        self.0.open.set(true);
        self.0.released.notify_waiters();
    }
}

/// The in-memory tree store: the contract's semantics with nothing durable.
/// One store may hold any number of trees, and it records every save. Its
/// blob namespace is in memory too.
pub struct MemoryTreeStore {
    stored: Rc<RefCell<Stored>>,
    blobs: Rc<dyn BlobStore>,
}

impl MemoryTreeStore {
    /// A store with a blob namespace of its own.
    pub fn new() -> Rc<Self> {
        Self::with_blobs(MemoryBlobs::new())
    }

    /// A store whose blob namespace is `blobs`, which a test reads or makes
    /// lose a blob.
    pub fn with_blobs(blobs: Rc<dyn BlobStore>) -> Rc<Self> {
        Rc::new(Self {
            stored: Rc::default(),
            blobs,
        })
    }

    /// Another store holding what this one holds now, as a process that died
    /// at this moment left it.
    pub fn copy(&self) -> Rc<Self> {
        Rc::new(Self {
            stored: Rc::new(RefCell::new(self.stored.borrow().clone())),
            blobs: self.blobs.clone(),
        })
    }

    /// Records what the conversation holds of `command`'s output, as the
    /// product's keeper does when the command ends.
    pub fn keep_output(&self, command: CommandId, output: StoredOutput) {
        self.stored.borrow_mut().outputs.insert(command, output);
    }

    /// Every save so far, in order, with the node it was for.
    pub fn saves(&self) -> Vec<(NodeId, CheckpointUpdate)> {
        self.stored.borrow().saves.clone()
    }

    /// The node's checkpoint as the store keeps it.
    pub fn checkpoint(&self, id: &NodeId) -> Option<Checkpoint> {
        load(&self.stored.borrow(), id).ok().flatten()
    }

    pub fn record(&self, id: &NodeId) -> Option<NodeRecord> {
        self.stored
            .borrow()
            .nodes
            .get(id)
            .map(|node| node.record.clone())
    }

    /// The node of the agent the model knows by `number`.
    pub fn numbered(&self, number: u64) -> Option<NodeId> {
        self.stored
            .borrow()
            .nodes
            .values()
            .find(|node| node.record.number == number)
            .map(|node| node.record.id.clone())
    }

    /// Refuses the next `count` saves, as a failing database would.
    pub fn fail_saves(&self, count: usize) {
        self.stored.borrow_mut().failing_saves = count;
    }

    /// Holds every save from now on until the gate is released.
    pub fn hold_saves(&self) -> StoreGate {
        let hold = Rc::new(Hold::default());
        self.stored.borrow_mut().save_hold = Some(hold.clone());
        StoreGate(hold)
    }

    /// Holds every number the store gives out from now on until the gate is
    /// released.
    pub fn hold_numbers(&self) -> StoreGate {
        let hold = Rc::new(Hold::default());
        self.stored.borrow_mut().number_hold = Some(hold.clone());
        StoreGate(hold)
    }

    /// Holds every read of `parent`'s children from now on until the gate
    /// is released.
    pub fn hold_children_of(&self, parent: &NodeId) -> StoreGate {
        let hold = Rc::new(Hold::default());
        self.stored
            .borrow_mut()
            .children_holds
            .insert(parent.clone(), hold.clone());
        StoreGate(hold)
    }
}

fn missing(id: &NodeId) -> StoreError {
    StoreError::Failed(format!("no node {id}"))
}

fn load(stored: &Stored, id: &NodeId) -> Result<Option<Checkpoint>, StoreError> {
    let Some(node) = stored.nodes.get(id) else {
        return Ok(None);
    };
    let transcript = (0..node.block_count)
        .map(|index| {
            node.blocks
                .get(&index)
                .cloned()
                .ok_or_else(|| StoreError::Corrupt(format!("node {id} has no block row {index}")))
        })
        .collect::<Result<_, _>>()?;
    Ok(Some(Checkpoint {
        state: node.state.clone(),
        transcript,
    }))
}

/// Applies a save to its node, and marks delivered the child rounds whose
/// completions it carries.
fn apply_save(
    stored: &mut Stored,
    id: &NodeId,
    update: CheckpointUpdate,
) -> Result<(), StoreError> {
    let completions = update.carried_completions()?;
    let node = stored.nodes.get_mut(id).ok_or_else(|| missing(id))?;
    for (index, block) in &update.changed_blocks {
        node.blocks.insert(*index, block.clone());
    }
    node.blocks.retain(|index, _| *index < update.block_count);
    node.block_count = update.block_count;
    node.state = update.state.clone();
    for round in completions {
        if let Some(child) = stored.nodes.get_mut(&round.child)
            && child.record.parent.as_ref() == Some(id)
            && child.record.round == round.round
        {
            child.record.delivered = true;
        }
    }
    stored.saves.push((id.clone(), update));
    Ok(())
}

/// One node's checkpoint store in a [`MemoryTreeStore`].
struct MemorySessionStore {
    stored: Rc<RefCell<Stored>>,
    blobs: Rc<dyn BlobStore>,
    id: NodeId,
}

impl SessionStore for MemorySessionStore {
    fn save(&self, update: CheckpointUpdate) -> LocalBoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            let hold = self.stored.borrow().save_hold.clone();
            if let Some(hold) = hold {
                hold.pass().await;
            }
            let mut stored = self.stored.borrow_mut();
            if stored.failing_saves > 0 {
                stored.failing_saves -= 1;
                return Err(StoreError::Failed(
                    "the database refused the save".to_owned(),
                ));
            }
            apply_save(&mut stored, &self.id, update)
        })
    }

    fn load(&self) -> LocalBoxFuture<'_, Result<Option<Checkpoint>, StoreError>> {
        Box::pin(async move { load(&self.stored.borrow(), &self.id) })
    }

    fn blobs(&self) -> &dyn BlobStore {
        &*self.blobs
    }
}

impl AgentTreeStore for MemoryTreeStore {
    fn node<'a>(
        &'a self,
        id: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<Option<NodeRecord>, StoreError>> {
        Box::pin(async move { Ok(self.record(id)) })
    }

    fn children<'a>(
        &'a self,
        parent: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<Vec<NodeRecord>, StoreError>> {
        Box::pin(async move {
            let hold = self.stored.borrow().children_holds.get(parent).cloned();
            if let Some(hold) = hold {
                hold.pass().await;
            }
            let stored = self.stored.borrow();
            let mut children: Vec<&StoredNode> = stored
                .nodes
                .values()
                .filter(|node| node.record.parent.as_ref() == Some(parent))
                .collect();
            children.sort_by_key(|node| node.record.number);
            Ok(children
                .into_iter()
                .map(|node| node.record.clone())
                .collect())
        })
    }

    fn create_node(
        &self,
        record: NodeRecord,
        initial: CheckpointUpdate,
    ) -> LocalBoxFuture<'_, Result<(), StoreError>> {
        Box::pin(async move {
            let mut stored = self.stored.borrow_mut();
            if stored.nodes.contains_key(&record.id) {
                return Err(StoreError::Failed(format!(
                    "node {} already exists",
                    record.id
                )));
            }
            let id = record.id.clone();
            stored.nodes.insert(
                id.clone(),
                StoredNode {
                    record,
                    state: initial.state.clone(),
                    blocks: BTreeMap::new(),
                    block_count: 0,
                },
            );
            apply_save(&mut stored, &id, initial)
        })
    }

    fn session_store(&self, id: &NodeId) -> Rc<dyn SessionStore> {
        Rc::new(MemorySessionStore {
            stored: self.stored.clone(),
            blobs: self.blobs.clone(),
            id: id.clone(),
        })
    }

    fn close_node<'a>(
        &'a self,
        id: &'a NodeId,
        close: NodeClose,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let mut stored = self.stored.borrow_mut();
            let node = stored.nodes.get_mut(id).ok_or_else(|| missing(id))?;
            node.record.closed = Some(close);
            node.record.delivered = false;
            Ok(())
        })
    }

    fn reopen_node<'a>(
        &'a self,
        id: &'a NodeId,
        round: u64,
        started_at: Timestamp,
        message: QueuedMessage,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let mut stored = self.stored.borrow_mut();
            let node = stored.nodes.get_mut(id).ok_or_else(|| missing(id))?;
            node.record.closed = None;
            node.record.delivered = false;
            node.record.round = round;
            node.record.started_at = started_at;
            node.state.queue = vec![message];
            Ok(())
        })
    }

    fn mark_delivered<'a>(
        &'a self,
        id: &'a NodeId,
        round: u64,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let mut stored = self.stored.borrow_mut();
            let node = stored.nodes.get_mut(id).ok_or_else(|| missing(id))?;
            if node.record.round == round {
                node.record.delivered = true;
            }
            Ok(())
        })
    }

    fn delete_node<'a>(&'a self, id: &'a NodeId) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let mut stored = self.stored.borrow_mut();
            let mut doomed = vec![id.clone()];
            let mut index = 0;
            while let Some(parent) = doomed.get(index).cloned() {
                doomed.extend(
                    stored
                        .nodes
                        .values()
                        .filter(|node| node.record.parent.as_ref() == Some(&parent))
                        .map(|node| node.record.id.clone()),
                );
                index += 1;
            }
            for doomed in doomed {
                stored.nodes.remove(&doomed);
            }
            Ok(())
        })
    }

    fn next_number(&self, sequence: Sequence) -> LocalBoxFuture<'_, Result<u64, StoreError>> {
        Box::pin(async move {
            let hold = self.stored.borrow().number_hold.clone();
            if let Some(hold) = hold {
                hold.pass().await;
            }
            let mut stored = self.stored.borrow_mut();
            let next = stored.sequences.entry(sequence).or_insert(1);
            let number = *next;
            *next += 1;
            Ok(number)
        })
    }

    fn command_output<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Result<Option<StoredOutput>, StoreError>> {
        Box::pin(async move { Ok(self.stored.borrow().outputs.get(command).cloned()) })
    }
}

/// A blob namespace in memory, naming bytes by their SHA-256 as a real one
/// does.
#[derive(Debug, Default)]
pub struct MemoryBlobs {
    blobs: RefCell<BTreeMap<BlobRef, B64Bytes>>,
}

impl MemoryBlobs {
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// Whether the namespace holds `blob`.
    pub fn holds(&self, blob: &BlobRef) -> bool {
        self.blobs.borrow().contains_key(blob)
    }

    /// Loses `blob`, as a namespace whose file went missing would.
    pub fn forget(&self, blob: &BlobRef) {
        self.blobs.borrow_mut().remove(blob);
    }
}

impl BlobStore for MemoryBlobs {
    fn put(&self, bytes: B64Bytes) -> LocalBoxFuture<'_, Result<BlobRef, StoreError>> {
        let blob = BlobRef::of(bytes.as_bytes());
        self.blobs.borrow_mut().insert(blob.clone(), bytes);
        Box::pin(async move { Ok(blob) })
    }

    fn get<'a>(
        &'a self,
        blob: &'a BlobRef,
    ) -> LocalBoxFuture<'a, Result<Option<B64Bytes>, StoreError>> {
        let bytes = self.blobs.borrow().get(blob).cloned();
        Box::pin(async move { Ok(bytes) })
    }
}

/// The tree store contract's cases (`subagents.md` § Persistence,
/// `runtime.md` § Tree store), for any realization of [`AgentTreeStore`]:
/// create queues the first message with the node, a save delivers the
/// completions it carries, reopen and delete, a completion of an earlier
/// round marks nothing delivered, and the blob namespace names bytes by
/// their SHA-256. Each case
/// takes a store that holds nothing yet and reads back only through the
/// contract, so the agent's in-memory store and the backend's database pass
/// the same cases.
pub mod store_contract {
    use demi_shared_types::{
        AgentMessage, AgentMessageBlock, AgentMessageEvent, B64Bytes, BlobRef, Block, CompletionId,
        CompletionOutcome, NodeId, QueuedMessage, Sender, Sequence, SessionPhase, Timestamp,
        TurnId, UserBlock,
    };

    use sha2::{Digest, Sha256};

    use super::{test_model, text};
    use crate::{
        AgentTreeStore, CheckpointState, CheckpointUpdate, ClosePhase, NodeClose, NodeRecord,
        PendingAgentInput,
    };

    fn id(value: &str) -> NodeId {
        NodeId::try_from(value).expect("a test node id is not empty")
    }

    /// A node in its first round, agent `number` of the conversation.
    fn record(node: &str, parent: Option<&str>, number: u64) -> NodeRecord {
        NodeRecord {
            id: id(node),
            number,
            parent: parent.map(id),
            description: String::new(),
            profile: None,
            round: 1,
            started_at: Timestamp::UNIX_EPOCH,
            can_spawn_subagents: true,
            closed: None,
            delivered: false,
        }
    }

    fn update(queue: Vec<QueuedMessage>, blocks: Vec<Block>) -> CheckpointUpdate {
        CheckpointUpdate {
            state: CheckpointState {
                phase: SessionPhase::Idle,
                queue,
                agent_inputs: Vec::new(),
                wakeups: Vec::new(),
                cwd: "/w".into(),
                model: test_model(),
                edits: Vec::new(),
            },
            block_count: blocks.len(),
            changed_blocks: blocks.into_iter().enumerate().collect(),
        }
    }

    fn message(turn: &str) -> QueuedMessage {
        QueuedMessage {
            id: TurnId::try_from(turn).expect("a test turn id is not empty"),
            content: text("brief"),
        }
    }

    /// The receipt a parent's transcript holds for a child's completed round.
    fn receipt(child: &str, round: u64) -> Block {
        let completion = CompletionId {
            child: id(child),
            round,
        };
        let message = AgentMessage {
            id: completion.block_id(),
            sender: Sender {
                id: id(child),
                number: 1,
                description: child.into(),
                round,
            },
            recipient_id: id("root"),
            timestamp: Timestamp::UNIX_EPOCH,
            content: format!("{child} done"),
            event: AgentMessageEvent::Completion {
                outcome: CompletionOutcome::Completed,
            },
        };
        Block::AgentMessage(AgentMessageBlock {
            id: completion.block_id(),
            turn_id: TurnId::try_from("turn").expect("a test turn id is not empty"),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            message,
        })
    }

    fn completed(result: &str) -> NodeClose {
        NodeClose {
            phase: ClosePhase::Completed {
                result: result.into(),
            },
            at: Timestamp::UNIX_EPOCH,
        }
    }

    async fn stored(store: &dyn AgentTreeStore, node: &str) -> Option<NodeRecord> {
        store
            .node(&id(node))
            .await
            .expect("the store reads its nodes")
    }

    async fn delivered(store: &dyn AgentTreeStore, node: &str) -> bool {
        stored(store, node)
            .await
            .expect("the node exists")
            .delivered
    }

    async fn queue(store: &dyn AgentTreeStore, node: &str) -> Vec<QueuedMessage> {
        store
            .session_store(&id(node))
            .load()
            .await
            .expect("the store reads its checkpoints")
            .expect("the node has a checkpoint")
            .state
            .queue
    }

    async fn create(store: &dyn AgentTreeStore, node: NodeRecord, initial: CheckpointUpdate) {
        store
            .create_node(node, initial)
            .await
            .expect("a new node is created");
    }

    pub async fn create_queues_the_first_message_with_the_node_and_a_save_replaces_it(
        store: &dyn AgentTreeStore,
    ) {
        create(
            store,
            record("root", None, 0),
            update(Vec::new(), Vec::new()),
        )
        .await;
        create(
            store,
            record("child", Some("root"), 1),
            update(vec![message("m1")], Vec::new()),
        )
        .await;

        let children: Vec<NodeId> = store
            .children(&id("root"))
            .await
            .expect("the store lists children")
            .into_iter()
            .map(|node| node.id)
            .collect();
        assert_eq!(children, [id("child")]);
        assert_eq!(queue(store, "child").await, [message("m1")]);
        let user = Block::User(UserBlock {
            id: "u1".try_into().expect("a test block id is not empty"),
            turn_id: TurnId::try_from("m1").expect("a test turn id is not empty"),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            content: text("brief"),
            preamble: None,
        });
        let child = store.session_store(&id("child"));
        child
            .save(update(Vec::new(), vec![user]))
            .await
            .expect("the save commits");
        let loaded = child
            .load()
            .await
            .expect("the store reads its checkpoints")
            .expect("the node has a checkpoint");
        assert!(loaded.state.queue.is_empty());
        assert_eq!(loaded.transcript.len(), 1);
        let again = store
            .create_node(
                record("child", Some("root"), 2),
                update(Vec::new(), Vec::new()),
            )
            .await;
        assert!(again.is_err(), "a node that exists is refused");
    }

    pub async fn a_save_delivers_the_completions_its_transcript_carries_and_only_those(
        store: &dyn AgentTreeStore,
    ) {
        create(
            store,
            record("root", None, 0),
            update(Vec::new(), Vec::new()),
        )
        .await;
        for (child, number) in [("a", 1), ("b", 2)] {
            create(
                store,
                record(child, Some("root"), number),
                update(Vec::new(), Vec::new()),
            )
            .await;
            store
                .close_node(&id(child), completed(&format!("{child} done")))
                .await
                .expect("the node closes");
        }

        let save = update(Vec::new(), vec![receipt("a", 1)]);
        assert_eq!(
            save.carried_completions()
                .expect("the receipt names a round"),
            [CompletionId {
                child: id("a"),
                round: 1
            }]
        );
        store
            .session_store(&id("root"))
            .save(save)
            .await
            .expect("the save commits");

        assert!(delivered(store, "a").await);
        assert!(!delivered(store, "b").await);
        store
            .mark_delivered(&id("b"), 1)
            .await
            .expect("the delivery is marked");
        assert!(delivered(store, "b").await);
    }

    pub async fn reopen_makes_a_closed_node_live_with_its_message_and_delete_takes_the_subtree(
        store: &dyn AgentTreeStore,
    ) {
        create(
            store,
            record("root", None, 0),
            update(Vec::new(), Vec::new()),
        )
        .await;
        create(
            store,
            record("child", Some("root"), 1),
            update(Vec::new(), Vec::new()),
        )
        .await;
        create(
            store,
            record("grandchild", Some("child"), 2),
            update(Vec::new(), Vec::new()),
        )
        .await;
        let failed = NodeClose {
            phase: ClosePhase::Error {
                failure: "boom".into(),
            },
            at: Timestamp::UNIX_EPOCH,
        };
        store
            .close_node(&id("child"), failed.clone())
            .await
            .expect("the node closes");
        assert_eq!(
            stored(store, "child")
                .await
                .expect("the node exists")
                .closed,
            Some(failed)
        );

        let resumed = Timestamp::from_millisecond(60_000).expect("the time is in range");
        store
            .reopen_node(&id("child"), 2, resumed, message("m2"))
            .await
            .expect("the node reopens");
        let reopened = stored(store, "child").await.expect("the node exists");
        assert_eq!(
            (
                reopened.closed,
                reopened.round,
                reopened.started_at,
                reopened.delivered
            ),
            (None, 2, resumed, false)
        );
        assert_eq!(queue(store, "child").await, [message("m2")]);

        store
            .delete_node(&id("child"))
            .await
            .expect("the subtree is deleted");
        assert!(stored(store, "child").await.is_none());
        assert!(stored(store, "grandchild").await.is_none());
        assert!(stored(store, "root").await.is_some());
    }

    pub async fn a_completion_of_an_earlier_round_marks_the_current_one_undelivered(
        store: &dyn AgentTreeStore,
    ) {
        create(
            store,
            record("root", None, 0),
            update(Vec::new(), Vec::new()),
        )
        .await;
        create(
            store,
            record("child", Some("root"), 1),
            update(Vec::new(), Vec::new()),
        )
        .await;
        store
            .close_node(&id("child"), completed("first"))
            .await
            .expect("the node closes");
        store
            .reopen_node(&id("child"), 2, Timestamp::UNIX_EPOCH, message("again"))
            .await
            .expect("the node reopens");
        store
            .close_node(&id("child"), completed("second"))
            .await
            .expect("the node closes");

        let root = store.session_store(&id("root"));
        let old = update(Vec::new(), vec![receipt("child", 1)]);
        root.save(old).await.expect("the save commits");
        store
            .mark_delivered(&id("child"), 1)
            .await
            .expect("an earlier round marks nothing");
        assert!(!delivered(store, "child").await);

        let current = update(Vec::new(), vec![receipt("child", 1), receipt("child", 2)]);
        root.save(current).await.expect("the save commits");
        assert!(delivered(store, "child").await);
    }

    /// Each sequence gives its numbers from 1, once each and in order, apart
    /// from the others.
    pub async fn each_sequence_gives_its_numbers_once_in_order(store: &dyn AgentTreeStore) {
        let mut given = Vec::new();
        for sequence in [
            Sequence::Command,
            Sequence::Command,
            Sequence::Shell,
            Sequence::Agent,
            Sequence::Command,
            Sequence::Shell,
        ] {
            let number = store
                .next_number(sequence)
                .await
                .expect("the store gives a number");
            given.push((sequence, number));
        }
        assert_eq!(
            given,
            [
                (Sequence::Command, 1),
                (Sequence::Command, 2),
                (Sequence::Shell, 1),
                (Sequence::Agent, 1),
                (Sequence::Command, 3),
                (Sequence::Shell, 2),
            ]
        );
    }

    /// A node's blob namespace names bytes by their SHA-256 and gives them
    /// back; a blob it does not hold reads as none.
    pub async fn the_blob_namespace_names_bytes_by_their_sha256(store: &dyn AgentTreeStore) {
        let blobs = store.session_store(&id("root"));
        let blobs = blobs.blobs();
        let bytes = B64Bytes::new(vec![0x89, b'P', b'N', b'G', 0, 1, 2, 3]);
        let name = format!("{:x}", Sha256::digest(bytes.as_bytes()));
        let blob = blobs
            .put(bytes.clone())
            .await
            .expect("the namespace stores bytes");
        assert_eq!(blob.as_str(), name);
        assert_eq!(
            blobs.get(&blob).await.expect("the namespace reads bytes"),
            Some(bytes)
        );
        let unknown = BlobRef::try_from("0".repeat(64)).expect("64 hexadecimal digits name a blob");
        assert_eq!(
            blobs
                .get(&unknown)
                .await
                .expect("the namespace reads bytes"),
            None
        );
    }

    /// A save delivers a completion it holds as waiting input as well as one
    /// its transcript holds.
    pub async fn a_save_delivers_a_completion_it_holds_as_waiting_input(
        store: &dyn AgentTreeStore,
    ) {
        create(
            store,
            record("root", None, 0),
            update(Vec::new(), Vec::new()),
        )
        .await;
        create(
            store,
            record("child", Some("root"), 1),
            update(Vec::new(), Vec::new()),
        )
        .await;
        store
            .close_node(&id("child"), completed("child done"))
            .await
            .expect("the node closes");
        let Block::AgentMessage(receipt) = receipt("child", 1) else {
            unreachable!("a receipt is an agent message")
        };
        let mut save = update(Vec::new(), Vec::new());
        save.state.agent_inputs.push(PendingAgentInput {
            turn_id: TurnId::try_from("waiting").expect("a test turn id is not empty"),
            model: test_model(),
            message: receipt.message,
        });
        store
            .session_store(&id("root"))
            .save(save)
            .await
            .expect("the save commits");
        assert!(delivered(store, "child").await);
    }

    /// A node's children list in spawn order, the order of their numbers,
    /// whatever order the store received them in; a close keeps its phase,
    /// time and result.
    pub async fn children_list_in_spawn_order_and_a_close_keeps_its_result(
        store: &dyn AgentTreeStore,
    ) {
        create(
            store,
            record("root", None, 0),
            update(Vec::new(), Vec::new()),
        )
        .await;
        for (child, number) in [("late", 4), ("early", 1), ("second", 3), ("first", 2)] {
            create(
                store,
                record(child, Some("root"), number),
                update(Vec::new(), Vec::new()),
            )
            .await;
        }
        let close = NodeClose {
            phase: ClosePhase::Completed {
                result: "found it".into(),
            },
            at: Timestamp::from_millisecond(90_000).expect("the time is in range"),
        };
        store
            .close_node(&id("early"), close.clone())
            .await
            .expect("the node closes");

        let children: Vec<String> = store
            .children(&id("root"))
            .await
            .expect("the store lists children")
            .into_iter()
            .map(|node| node.id.to_string())
            .collect();
        assert_eq!(children, ["early", "first", "second", "late"]);
        let early = stored(store, "early").await.expect("the node exists");
        assert_eq!((early.closed, early.delivered), (Some(close), false));
    }
}
