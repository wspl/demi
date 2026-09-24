//! Test support (feature `testing`): an in-memory tree store with the
//! contract's semantics, predictable identities, a model selection, and a
//! client that drives one connection the way a socket would. No test calls a
//! real model.

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};

use demi_agent_protocol::{ClientContent, ClientFrame, ServerFrame};
use demi_core::{
    B64Bytes, BlobRef, Block, Clock, Model, ModelSelection, NodeId, QueuedMessage, Timestamp,
    UserContentBlock,
};
use futures_util::future::LocalBoxFuture;
use sha2::{Digest, Sha256};

use demi_shell::{Host, HostError, HostFs, HostIdentity, HostKey, HostProcess, ShellEnvironment};

use crate::{
    AgentHarness, AgentServer, AgentTreeStore, Connection, ContentError, ContentResolver,
    EnvironmentScope, FileReference, FrameRx, IdSource, Outgoing, SessionStore,
    ShellEnvironmentFactory,
    store::{
        Checkpoint, CheckpointState, CheckpointUpdate, CommandStateSnapshot, CommitGuard,
        NodeClose, NodeRecord, StoreError,
        media::{self, BlobStore},
    },
};

/// The Host type of a test harness whose agents run no shell tools: it has
/// no values, so no environment is ever made on one.
#[derive(Debug)]
pub enum NoHost {}

impl Host for NoHost {
    fn key(&self) -> HostKey {
        match *self {}
    }

    fn default_cwd(&self) -> &str {
        match *self {}
    }

    fn identity(&self) -> HostIdentity {
        match *self {}
    }

    fn fs(&self) -> &dyn HostFs {
        match *self {}
    }

    fn process(&self) -> &dyn HostProcess {
        match *self {}
    }
}

/// The shell environment factory of a harness whose Host is [`NoHost`].
#[derive(Debug, Default)]
pub struct NoShells;

impl ShellEnvironmentFactory<NoHost> for NoShells {
    fn create<'a>(
        &'a self,
        _: EnvironmentScope<'a>,
        host: Rc<NoHost>,
    ) -> LocalBoxFuture<'a, Result<Rc<dyn ShellEnvironment>, HostError>> {
        match *host {}
    }
}

/// Identities `<prefix>-1`, `<prefix>-2`, and on.
#[derive(Debug)]
pub struct SequentialIds {
    prefix: String,
    next: Cell<u64>,
}

impl SequentialIds {
    pub fn new(prefix: &str) -> Self {
        Self {
            prefix: prefix.to_owned(),
            next: Cell::new(1),
        }
    }
}

impl IdSource for SequentialIds {
    fn next_id(&self) -> String {
        let id = self.next.get();
        self.next.set(id + 1);
        format!("{}-{id}", self.prefix)
    }
}

/// A model selection of the provider `provider` with a 100,000-token window.
pub fn model_of(provider: &str, model: &str) -> ModelSelection {
    ModelSelection {
        provider_id: provider.to_owned(),
        model: Model {
            id: model.to_owned(),
            name: model.to_owned(),
            context_window: 100_000,
            input_limit: None,
            output_limit: None,
            thinking: Vec::new(),
            accepted_extensions: Some(Vec::new()),
        },
        thinking: None,
        service_tier_id: None,
    }
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

/// A message's content of one text, as the browser sends it.
pub fn client_text(text: &str) -> Vec<ClientContent> {
    vec![ClientContent::Text {
        text: text.to_owned(),
    }]
}

/// A wall clock that moves with Tokio's, so that a test on the paused clock
/// moves the times records carry too: `start` plus the Tokio time elapsed
/// since the clock was made.
#[derive(Debug)]
pub struct TokioClock {
    start: Timestamp,
    origin: tokio::time::Instant,
}

impl TokioClock {
    pub fn new(start: Timestamp) -> Self {
        Self {
            start,
            origin: tokio::time::Instant::now(),
        }
    }
}

impl Clock for TokioClock {
    fn now(&self) -> Timestamp {
        let elapsed = i64::try_from(self.origin.elapsed().as_millis()).unwrap_or(i64::MAX);
        Timestamp::from_millisecond(self.start.as_millisecond().saturating_add(elapsed))
            .expect("a test's time stays in range")
    }
}

/// Uploads a test gave the blocks they resolve to; every other file
/// reference is refused, as a backend refuses one it does not hold.
#[derive(Debug, Default)]
pub struct TestFiles {
    uploads: RefCell<BTreeMap<String, Vec<UserContentBlock>>>,
}

impl TestFiles {
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// The upload `reference` resolves to `blocks`.
    pub fn upload(&self, reference: &str, blocks: Vec<UserContentBlock>) {
        self.uploads
            .borrow_mut()
            .insert(reference.to_owned(), blocks);
    }
}

impl ContentResolver for TestFiles {
    fn resolve<'a>(
        &'a self,
        files: Vec<FileReference>,
    ) -> LocalBoxFuture<'a, Result<Vec<Vec<UserContentBlock>>, ContentError>> {
        Box::pin(async move {
            files
                .into_iter()
                .map(|file| match file {
                    FileReference::Upload { r#ref, .. } => self
                        .uploads
                        .borrow()
                        .get(&r#ref)
                        .cloned()
                        .ok_or(ContentError {
                            message: format!("upload {ref} is not available"),
                            code: Some("frame_delivery_failed".to_owned()),
                        }),
                    FileReference::RemoteFile { device_id, .. } => Err(ContentError {
                        message: format!("device {device_id} is not paired"),
                        code: Some("frame_delivery_failed".to_owned()),
                    }),
                })
                .collect()
        })
    }
}

/// One node as the store holds it.
#[derive(Debug, Clone)]
struct StoredNode {
    record: NodeRecord,
    state: CheckpointState,
    command_state: CommandStateSnapshot,
    blocks: BTreeMap<usize, Block>,
    block_count: usize,
    /// Its place in creation order, which breaks ties of equal rounds.
    created: u64,
}

#[derive(Debug, Default, Clone)]
struct Stored {
    nodes: BTreeMap<NodeId, StoredNode>,
    saves: Vec<(NodeId, CheckpointUpdate)>,
    created: u64,
    failing_saves: usize,
    hold: Option<Rc<Hold>>,
}

/// Saves waiting until a test lets them commit.
#[derive(Debug, Default)]
struct Hold {
    waiting: Cell<usize>,
    open: Cell<bool>,
    released: tokio::sync::Notify,
}

/// A hold on a [`MemoryTreeStore`]'s saves: each waits until
/// [`release`](Self::release), as a save behind a slow database would.
#[derive(Debug)]
pub struct SaveGate(Rc<Hold>);

impl SaveGate {
    /// How many saves wait.
    pub fn waiting(&self) -> usize {
        self.0.waiting.get()
    }

    /// Lets the waiting saves and every later one commit.
    pub fn release(&self) {
        self.0.open.set(true);
        self.0.released.notify_waiters();
    }
}

/// The in-memory tree store: the contract's semantics with nothing durable.
/// One store may hold any number of trees, and it records every save. Given
/// a blob namespace, it keeps media by reference as a product's store does.
#[derive(Default)]
pub struct MemoryTreeStore {
    stored: Rc<RefCell<Stored>>,
    blobs: Option<Rc<dyn BlobStore>>,
}

impl MemoryTreeStore {
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// A store that moves media into `blobs` before it saves a block and
    /// puts the bytes back when it loads one.
    pub fn with_blobs(blobs: Rc<dyn BlobStore>) -> Rc<Self> {
        Rc::new(Self {
            stored: Rc::default(),
            blobs: Some(blobs),
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

    /// Every save so far, in order, with the node it was for.
    pub fn saves(&self) -> Vec<(NodeId, CheckpointUpdate)> {
        self.stored.borrow().saves.clone()
    }

    /// The node's checkpoint as the store keeps it: media by reference when
    /// the store has a blob namespace.
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

    /// Refuses the next `count` saves, as a failing database would.
    pub fn fail_saves(&self, count: usize) {
        self.stored.borrow_mut().failing_saves = count;
    }

    /// Holds every save from now on until the gate is released.
    pub fn hold_saves(&self) -> SaveGate {
        let hold = Rc::new(Hold::default());
        self.stored.borrow_mut().hold = Some(hold.clone());
        SaveGate(hold)
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
        command_state: node.command_state.clone(),
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
    if let Some(command_state) = &update.command_state {
        for version in &command_state.versions {
            let changed = node.command_state.versions.iter().any(|existing| {
                existing.revision == version.revision && existing.values != version.values
            });
            if changed {
                return Err(StoreError::Failed(format!(
                    "command-state version {} is immutable",
                    version.revision
                )));
            }
        }
        node.command_state = command_state.clone();
    }
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

/// Moves the inline media of `update`'s blocks into `blobs`, when there are
/// any.
async fn externalized(
    mut update: CheckpointUpdate,
    blobs: Option<&dyn BlobStore>,
) -> Result<CheckpointUpdate, StoreError> {
    if let Some(blobs) = blobs {
        for (_, block) in &mut update.changed_blocks {
            media::externalize(block, blobs).await?;
        }
    }
    Ok(update)
}

/// One node's checkpoint store in a [`MemoryTreeStore`].
struct MemorySessionStore {
    stored: Rc<RefCell<Stored>>,
    blobs: Option<Rc<dyn BlobStore>>,
    id: NodeId,
}

impl SessionStore for MemorySessionStore {
    fn save<'a>(
        &'a self,
        update: CheckpointUpdate,
        guard: &'a CommitGuard,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let update = externalized(update, self.blobs.as_deref()).await?;
            let hold = self.stored.borrow().hold.clone();
            if let Some(hold) = hold {
                hold.waiting.set(hold.waiting.get() + 1);
                while !hold.open.get() {
                    let released = hold.released.notified();
                    tokio::pin!(released);
                    released.as_mut().enable();
                    if hold.open.get() {
                        break;
                    }
                    released.await;
                }
                hold.waiting.set(hold.waiting.get() - 1);
            }
            guard.check()?;
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
        Box::pin(async move {
            let loaded = load(&self.stored.borrow(), &self.id)?;
            let (Some(mut checkpoint), Some(blobs)) = (loaded.clone(), &self.blobs) else {
                return Ok(loaded);
            };
            for block in &mut checkpoint.transcript {
                media::rehydrate(block, &**blobs).await?;
            }
            Ok(Some(checkpoint))
        })
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
            let stored = self.stored.borrow();
            let mut children: Vec<&StoredNode> = stored
                .nodes
                .values()
                .filter(|node| node.record.parent.as_ref() == Some(parent))
                .collect();
            children.sort_by_key(|node| (node.record.round, node.created));
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
            let initial = externalized(initial, self.blobs.as_deref()).await?;
            let mut stored = self.stored.borrow_mut();
            if stored.nodes.contains_key(&record.id) {
                return Err(StoreError::Failed(format!(
                    "node {} already exists",
                    record.id
                )));
            }
            let created = stored.created;
            stored.created += 1;
            let id = record.id.clone();
            stored.nodes.insert(
                id.clone(),
                StoredNode {
                    record,
                    state: initial.state.clone(),
                    command_state: CommandStateSnapshot::initial(),
                    blocks: BTreeMap::new(),
                    block_count: 0,
                    created,
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
        message: QueuedMessage,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let mut stored = self.stored.borrow_mut();
            let node = stored.nodes.get_mut(id).ok_or_else(|| missing(id))?;
            node.record.closed = None;
            node.record.delivered = false;
            node.record.round = round;
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
        let name = format!("{:x}", Sha256::digest(bytes.as_bytes()));
        let blob = BlobRef::try_from(name).expect("a SHA-256 in hexadecimal names a blob");
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

/// A client of one connection: it hands frames to the connection as the
/// backend's socket task would, and reads what the outbox sends back.
pub struct TestClient<H: AgentHarness> {
    connection: Connection<H>,
    frames: FrameRx,
}

impl<H: AgentHarness> TestClient<H> {
    /// A client of the conversation `root`, whose new tree works in `cwd`,
    /// whose frames refer to no file.
    pub fn connect(server: &Rc<AgentServer<H>>, root: &NodeId, cwd: &str) -> Self {
        Self::connect_with(server, root, cwd, TestFiles::new())
    }

    /// A client whose frames' files `files` resolves.
    pub fn connect_with(
        server: &Rc<AgentServer<H>>,
        root: &NodeId,
        cwd: &str,
        files: Rc<dyn ContentResolver>,
    ) -> Self {
        let (connection, frames) = server.connect(root.clone(), cwd.to_owned(), files);
        Self { connection, frames }
    }

    /// Hands `frame` to the connection and waits until it is handled.
    pub async fn send(&self, frame: ClientFrame) {
        self.connection.handle(frame).await;
    }

    /// The next frame; `None` once the outbox is closed or lagged.
    pub async fn next(&mut self) -> Option<ServerFrame> {
        match self.frames.recv().await {
            Outgoing::Frame(frame) => Some(frame),
            Outgoing::Lagged | Outgoing::Closed => None,
        }
    }

    /// Every frame waiting now.
    pub fn received(&mut self) -> Vec<ServerFrame> {
        let mut frames = Vec::new();
        while let Some(Outgoing::Frame(frame)) = self.frames.try_recv() {
            frames.push(frame);
        }
        frames
    }

    /// The frames up to and including the first that `until` accepts.
    pub async fn next_until(&mut self, until: impl Fn(&ServerFrame) -> bool) -> Vec<ServerFrame> {
        let mut frames = Vec::new();
        while let Some(frame) = self.next().await {
            let done = until(&frame);
            frames.push(frame);
            if done {
                break;
            }
        }
        frames
    }

    /// What the outbox has next, lagged or closed included.
    pub async fn outgoing(&mut self) -> Outgoing {
        self.frames.recv().await
    }

    pub fn connection(&self) -> &Connection<H> {
        &self.connection
    }
}

/// The tree store contract's cases (`subagents.md` § Persistence), for any
/// realization of [`AgentTreeStore`]: create queues the first message with
/// the node, a save delivers the completions it carries, reopen and delete,
/// and a completion of an earlier round marks nothing delivered. Each case
/// takes a store that holds nothing yet and reads back only through the
/// contract, so the agent's in-memory store and the backend's database pass
/// the same cases.
pub mod store_contract {
    use demi_core::{
        AgentMessage, AgentMessageBlock, AgentMessageEvent, B64Bytes, BlobRef, Block, CompletionId,
        CompletionOutcome, MediaSource, NodeId, QueuedMessage, Sender, SessionPhase, Timestamp,
        TurnId, UserBlock, UserContentBlock,
    };

    use sha2::{Digest, Sha256};

    use super::{test_model, text};
    use crate::{
        AgentTreeStore,
        store::{
            CheckpointState, CheckpointUpdate, ClosePhase, CommandStateSnapshot, NodeClose,
            NodeRecord, PendingAgentInput, media::BlobStore,
        },
    };

    fn id(value: &str) -> NodeId {
        NodeId::try_from(value).expect("a test node id is not empty")
    }

    fn record(node: &str, parent: Option<&str>, round: u64) -> NodeRecord {
        NodeRecord {
            id: id(node),
            parent: parent.map(id),
            description: String::new(),
            profile: None,
            round,
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
                harness: "test".into(),
                edits: Vec::new(),
            },
            command_state: Some(CommandStateSnapshot::initial()),
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
            record("root", None, 1),
            update(Vec::new(), Vec::new()),
        )
        .await;
        create(
            store,
            record("child", Some("root"), 2),
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
            .save(update(Vec::new(), vec![user]), &Default::default())
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
                record("child", Some("root"), 3),
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
            record("root", None, 1),
            update(Vec::new(), Vec::new()),
        )
        .await;
        for child in ["a", "b"] {
            create(
                store,
                record(child, Some("root"), 1),
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
            .save(save, &Default::default())
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
            record("root", None, 1),
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
            record("grandchild", Some("child"), 1),
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

        store
            .reopen_node(&id("child"), 9, message("m2"))
            .await
            .expect("the node reopens");
        let reopened = stored(store, "child").await.expect("the node exists");
        assert_eq!(
            (reopened.closed, reopened.round, reopened.delivered),
            (None, 9, false)
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
            record("root", None, 1),
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
            .reopen_node(&id("child"), 3, message("again"))
            .await
            .expect("the node reopens");
        store
            .close_node(&id("child"), completed("second"))
            .await
            .expect("the node closes");

        let root = store.session_store(&id("root"));
        let old = update(Vec::new(), vec![receipt("child", 1)]);
        root.save(old, &Default::default())
            .await
            .expect("the save commits");
        store
            .mark_delivered(&id("child"), 1)
            .await
            .expect("an earlier round marks nothing");
        assert!(!delivered(store, "child").await);

        let current = update(Vec::new(), vec![receipt("child", 1), receipt("child", 3)]);
        root.save(current, &Default::default())
            .await
            .expect("the save commits");
        assert!(delivered(store, "child").await);
    }

    /// A store with the blob namespace `blobs` keeps a block's media there
    /// and puts the bytes back when it loads it; a blob that `forget` lost
    /// loads as the text that names it.
    pub async fn media_travels_by_reference(
        store: &dyn AgentTreeStore,
        blobs: &dyn BlobStore,
        forget: &dyn Fn(&BlobRef),
    ) {
        let bytes = B64Bytes::new(vec![0x89, b'P', b'N', b'G', 0, 1, 2, 3, 4, 5, 6, 7]);
        let image = UserContentBlock::Image {
            source: MediaSource::Binary {
                data: bytes.clone(),
                media_type: "image/png".into(),
            },
        };
        let user = Block::User(UserBlock {
            id: "u1".try_into().expect("a test block id is not empty"),
            turn_id: TurnId::try_from("m1").expect("a test turn id is not empty"),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            content: vec![image.clone()],
            preamble: None,
        });
        create(
            store,
            record("root", None, 1),
            update(Vec::new(), vec![user]),
        )
        .await;
        let content = |checkpoint: Option<crate::store::Checkpoint>| {
            let checkpoint = checkpoint.expect("the node has a checkpoint");
            match &checkpoint.transcript[0] {
                Block::User(user) => user.content.clone(),
                other => panic!("{other:?} is not the user block"),
            }
        };
        let root = store.session_store(&id("root"));
        let loaded = root.load().await.expect("the store reads its checkpoints");
        assert_eq!(content(loaded), [image]);
        let name = format!("{:x}", Sha256::digest(bytes.as_bytes()));
        let blob = BlobRef::try_from(name).expect("a SHA-256 in hexadecimal names a blob");
        assert_eq!(
            blobs.get(&blob).await.expect("the namespace reads bytes"),
            Some(bytes),
            "the save published the bytes"
        );
        forget(&blob);
        let loaded = root.load().await.expect("the store reads its checkpoints");
        assert_eq!(
            content(loaded),
            [UserContentBlock::Text {
                text: format!("[missing image blob {blob}]")
            }]
        );
    }

    /// A save delivers a completion it holds as waiting input as well as one
    /// its transcript holds.
    pub async fn a_save_delivers_a_completion_it_holds_as_waiting_input(
        store: &dyn AgentTreeStore,
    ) {
        create(
            store,
            record("root", None, 1),
            update(Vec::new(), Vec::new()),
        )
        .await;
        create(
            store,
            record("child", Some("root"), 4),
            update(Vec::new(), Vec::new()),
        )
        .await;
        store
            .close_node(&id("child"), completed("child done"))
            .await
            .expect("the node closes");
        let Block::AgentMessage(receipt) = receipt("child", 4) else {
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
            .save(save, &Default::default())
            .await
            .expect("the save commits");
        assert!(delivered(store, "child").await);
    }

    /// A node's children list in spawn order, and in creation order for one
    /// spawn time; a close keeps its phase, time and result.
    pub async fn children_list_in_spawn_order_and_a_close_keeps_its_result(
        store: &dyn AgentTreeStore,
    ) {
        create(
            store,
            record("root", None, 1),
            update(Vec::new(), Vec::new()),
        )
        .await;
        for (child, round) in [
            ("late", 3),
            ("early", 1),
            ("first-of-two", 2),
            ("second-of-two", 2),
        ] {
            create(
                store,
                record(child, Some("root"), round),
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
        assert_eq!(children, ["early", "first-of-two", "second-of-two", "late"]);
        let early = stored(store, "early").await.expect("the node exists");
        assert_eq!((early.closed, early.delivered), (Some(close), false));
    }
}
