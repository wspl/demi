//! Test support (feature `testing`): an in-memory tree store with the
//! contract's semantics, predictable identities, a model selection, and a
//! client that drives one connection the way a socket would. No test calls a
//! real model.

use std::{
    cell::{Cell, RefCell},
    collections::BTreeMap,
    rc::Rc,
};

use demi_agent_protocol::ServerFrame;
use demi_core::{Block, Model, ModelSelection, NodeId, QueuedMessage, UserContentBlock};
use futures_util::future::LocalBoxFuture;

use crate::{
    AgentHarness, AgentServer, AgentTreeStore, Connection, FrameRx, IdSource, Outgoing,
    SessionStore,
    store::{
        Checkpoint, CheckpointState, CheckpointUpdate, CommandStateSnapshot, CommitGuard,
        NodeClose, NodeRecord, StoreError,
    },
};

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
}

/// The in-memory tree store: the contract's semantics with nothing durable.
/// One store may hold any number of trees, and it records every save.
#[derive(Debug, Default)]
pub struct MemoryTreeStore {
    stored: Rc<RefCell<Stored>>,
}

impl MemoryTreeStore {
    pub fn new() -> Rc<Self> {
        Rc::new(Self::default())
    }

    /// Another store holding what this one holds now, as a process that died
    /// at this moment left it.
    pub fn copy(&self) -> Rc<Self> {
        Rc::new(Self {
            stored: Rc::new(RefCell::new(self.stored.borrow().clone())),
        })
    }

    /// Every save so far, in order, with the node it was for.
    pub fn saves(&self) -> Vec<(NodeId, CheckpointUpdate)> {
        self.stored.borrow().saves.clone()
    }

    /// The node's checkpoint as a load would read it.
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

/// One node's checkpoint store in a [`MemoryTreeStore`].
struct MemorySessionStore {
    stored: Rc<RefCell<Stored>>,
    id: NodeId,
}

impl SessionStore for MemorySessionStore {
    fn save<'a>(
        &'a self,
        update: CheckpointUpdate,
        guard: &'a CommitGuard,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
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
        Box::pin(async move { load(&self.stored.borrow(), &self.id) })
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

/// A client of one connection: it hands frames to the connection as the
/// backend's socket task would, and reads what the outbox sends back.
pub struct TestClient<H: AgentHarness> {
    connection: Connection<H>,
    frames: FrameRx,
}

impl<H: AgentHarness> TestClient<H> {
    /// A client of the conversation `root`, whose new tree works in `cwd`.
    pub fn connect(server: &Rc<AgentServer<H>>, root: &NodeId, cwd: &str) -> Self {
        let (connection, frames) = server.connect(root.clone(), cwd.to_owned());
        Self { connection, frames }
    }

    /// Hands `frame` to the connection and waits until it is handled.
    pub async fn send(&self, frame: demi_agent_protocol::ClientFrame<UserContentBlock>) {
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
