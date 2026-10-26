//! An in-memory tree store with the contract's semantics, an in-memory blob
//! namespace, and the model selections, texts and images the compaction tests
//! build on (feature `testing`).

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    rc::Rc,
};

use demi_provider_common::UserPart;
use demi_shared_types::{
    B64Bytes, BlobRef, Block, BlockId, CommandEnd, CommandId, FileExtension, Model, ModelSelection, NodeId,
    QueuedMessage, Sequence, Timestamp, TurnId, UserContentBlock,
};
use futures_util::future::LocalBoxFuture;

use demi_conversation_lane_protocol::Versions;

use crate::{
    AgentTreeStore, Checkpoint, CheckpointState, CheckpointUpdate, ChildDirectory, EarlierRows, Kind, NodeClose,
    NodeRecord, Reprofile, Rows, RunningCommand, SessionStore, StoreError, StoredCommand, StoredPrefix, StoredSubagent, Switched, SwitchedBefore,
    media::BlobStore,
    versions::{Sibling, version_rows, versions_of},
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
/// transcript.
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
    /// The node's rows by their number, never removed.
    rows: Vec<StoredRow>,
    /// The current path: the row at each position.
    path: Vec<u64>,
    /// The row a cut left off the path's end that no row replaced yet.
    cut: Option<u64>,
}

/// One row of a node's tree. Its block is shared, so a node that starts
/// with another's rows shares their blocks, as the database copies rows
/// without the process holding them.
#[derive(Debug, Clone)]
struct StoredRow {
    block: Rc<Block>,
    follows: Option<u64>,
    replaces: Option<u64>,
}

impl StoredNode {
    fn row(&self, row: u64) -> &StoredRow {
        &self.rows[usize::try_from(row).expect("a row number fits")]
    }

    /// The blocks at the positions `range` of the path.
    fn blocks(&self, range: std::ops::Range<usize>) -> Vec<Block> {
        self.path[range].iter().map(|row| Block::clone(&self.row(*row).block)).collect()
    }

    /// The position of the path's first block of `kind` from `from` on.
    fn next_of(&self, kind: Kind, from: usize) -> Option<usize> {
        let from = from.min(self.path.len());
        self.path[from..]
            .iter()
            .position(|row| kind.holds(&self.row(*row).block))
            .map(|at| from + at)
    }

    /// What [`SessionStore::earlier`] reads before `end`.
    fn earlier(&self, end: usize) -> EarlierRows {
        let end = end.min(self.path.len());
        let mut rows = EarlierRows::default();
        for row in &self.path[..end] {
            match &*self.row(*row).block {
                Block::User(user) => rows.user_turns.push(user.turn_id.clone()),
                Block::ToolCall(call) => rows.calls.push(call.id.clone()),
                _ => {}
            }
        }
        let last = |kind: Kind| self.last_of(kind, end).map(|at| self.blocks(at..at + 1).remove(0));
        rows.last_input = last(Kind::Input);
        rows.last_user_input = last(Kind::UserInput);
        rows.last_record = last(Kind::Record);
        rows
    }

    /// The position of the path's last block of `kind` before `end`.
    fn last_of(&self, kind: Kind, end: usize) -> Option<usize> {
        self.path[..end.min(self.path.len())]
            .iter()
            .rposition(|row| kind.holds(&self.row(*row).block))
    }

    /// The row `row` was carried from through every compaction that
    /// carried it: one that replaces a row with another parent is that
    /// row's block on a new branch; `row` itself otherwise.
    fn carried(&self, row: u64) -> u64 {
        let mut row = row;
        while let Some(replaced) = self
            .row(row)
            .replaces
            .filter(|replaced| self.row(*replaced).follows != self.row(row).follows)
        {
            row = replaced;
        }
        row
    }

    /// The parent of the place whose versions `position` shows: its own,
    /// or, for a block a compaction carried whose own place holds none,
    /// the place it was carried from.
    fn shown_place(&self, position: usize, following: &Following) -> Option<u64> {
        let own = position.checked_sub(1).map(|before| self.path[before]);
        let row = self.path[position];
        if versions_of(following.of(own), row).is_some() {
            return own;
        }
        self.row(self.carried(row)).follows
    }

    /// The versions at each position of the path from `from` on.
    ///
    /// A place's versions are those at the block that stands there, or at the
    /// block a compaction carried from there, which the boundary then stands in
    /// for without showing them.
    fn versions(&self, from: usize) -> BTreeMap<usize, Versions> {
        let following = Following::of_rows(&self.rows);
        let mut shown: BTreeMap<Option<u64>, (usize, Versions)> = BTreeMap::new();
        for position in 0..self.path.len() {
            let parent = self.shown_place(position, &following);
            let row = self.carried(self.path[position]);
            if let Some(versions) = versions_of(following.of(parent), row) {
                shown.insert(parent, (position, versions));
            }
        }
        shown.into_values().filter(|(position, _)| *position >= from).collect()
    }
}

/// The rows that follow each row, read in one pass: asking each place of
/// a path for its siblings apart reads every row per place, which makes a
/// load of a long path (`tests/held`) minutes.
struct Following(BTreeMap<Option<u64>, Vec<Sibling>>);

impl Following {
    fn of_rows(rows: &[StoredRow]) -> Self {
        let mut following: BTreeMap<Option<u64>, Vec<Sibling>> = BTreeMap::new();
        for (number, row) in (0u64..).zip(rows) {
            following.entry(row.follows).or_default().push(Sibling {
                row: number,
                replaces: row.replaces,
            });
        }
        Self(following)
    }

    fn of(&self, parent: Option<u64>) -> &[Sibling] {
        self.0.get(&parent).map_or(&[], Vec::as_slice)
    }
}

#[derive(Debug, Default, Clone)]
struct Stored {
    nodes: BTreeMap<NodeId, StoredNode>,
    saves: Vec<(NodeId, CheckpointUpdate)>,
    /// The next number of each sequence that gave one out.
    sequences: BTreeMap<Sequence, u64>,
    /// What the product's keeper stored of each ended command's output.
    outputs: BTreeMap<CommandId, StoredCommand>,
    /// The commands each node ran that are recorded running.
    running: BTreeMap<NodeId, Vec<RunningCommand>>,
    /// How far each node looked in each running command's output.
    places: BTreeMap<(CommandId, NodeId), demi_host_interface::Seen>,
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

    /// Every save so far, in order, with the node it was for.
    pub fn saves(&self) -> Vec<(NodeId, CheckpointUpdate)> {
        self.stored.borrow().saves.clone()
    }

    /// The node's checkpoint as a load reads it: its window.
    pub fn checkpoint(&self, id: &NodeId) -> Option<Checkpoint> {
        load(&self.stored.borrow(), id).ok().flatten()
    }

    /// The blocks of the node's whole current path.
    pub fn path(&self, id: &NodeId) -> Vec<Block> {
        self.stored
            .borrow()
            .nodes
            .get(id)
            .map(|node| node.blocks(0..node.path.len()))
            .unwrap_or_default()
    }

    pub fn record(&self, id: &NodeId) -> Option<NodeRecord> {
        self.stored
            .borrow()
            .nodes
            .get(id)
            .map(|node| node.record.clone())
    }

}

fn missing(id: &NodeId) -> StoreError {
    StoreError::Failed(format!("no node {id}"))
}

fn load(stored: &Stored, id: &NodeId) -> Result<Option<Checkpoint>, StoreError> {
    let Some(node) = stored.nodes.get(id) else {
        return Ok(None);
    };
    let start = node.last_of(Kind::CompactionBoundary, node.path.len()).unwrap_or(0);
    Ok(Some(Checkpoint {
        state: node.state.clone(),
        start,
        transcript: node.blocks(start..node.path.len()),
        versions: node.versions(start),
    }))
}

/// The node `id` of `stored`, or the failure of a read of one it lacks.
fn node<'a>(stored: &'a Stored, id: &NodeId) -> Result<&'a StoredNode, StoreError> {
    stored.nodes.get(id).ok_or_else(|| missing(id))
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
    let stored_length = node.path.len();
    let cut = update
        .moved_from
        .unwrap_or(usize::MAX)
        .min(update.block_count)
        .min(stored_length);
    // A block carried to the new branch under its id replaces its row
    // there, whose versions it keeps.
    let mut moved: BTreeMap<BlockId, u64> = BTreeMap::new();
    if cut < stored_length {
        node.cut = Some(node.path[cut]);
        moved = node.path[cut..].iter().map(|row| (node.row(*row).block.id().clone(), *row)).collect();
        node.path.truncate(cut);
    }
    if update.version {
        node.cut = None;
    }
    for (position, block) in &update.changed_blocks {
        if *position < cut {
            let row = usize::try_from(node.path[*position]).expect("a row number fits");
            node.rows[row].block = Rc::new(block.clone());
            continue;
        }
        if *position != node.path.len() {
            return Err(StoreError::Failed(format!("a save of node {id} skips position {}", node.path.len())));
        }
        let follows = node.path.last().copied();
        let replaces = match moved.remove(block.id()) {
            Some(row) => {
                node.cut = None;
                Some(row)
            }
            None => node.cut.take().filter(|cut| node.row(*cut).follows == follows),
        };
        node.rows.push(StoredRow {
            block: Rc::new(block.clone()),
            follows,
            replaces,
        });
        node.path.push(u64::try_from(node.rows.len() - 1).expect("a row number fits"));
    }
    if node.path.len() != update.block_count {
        return Err(StoreError::Failed(format!("a save of node {id} leaves its path short")));
    }
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
            let mut stored = self.stored.borrow_mut();
            apply_save(&mut stored, &self.id, update)
        })
    }

    fn load(&self) -> LocalBoxFuture<'_, Result<Option<Checkpoint>, StoreError>> {
        Box::pin(async move { load(&self.stored.borrow(), &self.id) })
    }

    fn blocks(&self, range: std::ops::Range<usize>) -> LocalBoxFuture<'_, Result<Rows, StoreError>> {
        Box::pin(async move {
            let stored = self.stored.borrow();
            let node = node(&stored, &self.id)?;
            if range.end > node.path.len() {
                return Err(StoreError::Failed(format!("the path of node {} ends before {}", self.id, range.end)));
            }
            let mut versions = node.versions(range.start);
            versions.retain(|position, _| *position < range.end);
            Ok(Rows {
                blocks: node.blocks(range),
                versions,
            })
        })
    }

    fn position<'a>(&'a self, id: &'a BlockId) -> LocalBoxFuture<'a, Result<Option<usize>, StoreError>> {
        Box::pin(async move {
            let stored = self.stored.borrow();
            let node = node(&stored, &self.id)?;
            Ok(node.path.iter().position(|row| node.row(*row).block.id() == id))
        })
    }

    fn last_of(&self, kind: Kind, end: usize) -> LocalBoxFuture<'_, Result<Option<usize>, StoreError>> {
        Box::pin(async move { Ok(node(&self.stored.borrow(), &self.id)?.last_of(kind, end)) })
    }

    fn next_of(&self, kind: Kind, from: usize) -> LocalBoxFuture<'_, Result<Option<usize>, StoreError>> {
        Box::pin(async move { Ok(node(&self.stored.borrow(), &self.id)?.next_of(kind, from)) })
    }

    fn blocks_of(
        &self,
        kinds: Vec<Kind>,
        range: std::ops::Range<usize>,
    ) -> LocalBoxFuture<'_, Result<Vec<(usize, Block)>, StoreError>> {
        Box::pin(async move {
            let stored = self.stored.borrow();
            let node = node(&stored, &self.id)?;
            let end = range.end.min(node.path.len());
            Ok((range.start.min(end)..end)
                .filter_map(|at| {
                    let block = &node.row(node.path[at]).block;
                    kinds.iter().any(|kind| kind.holds(block)).then(|| (at, Block::clone(block)))
                })
                .collect())
        })
    }

    fn turn_position<'a>(&'a self, turn: &'a TurnId) -> LocalBoxFuture<'a, Result<Option<usize>, StoreError>> {
        Box::pin(async move {
            let stored = self.stored.borrow();
            let node = node(&stored, &self.id)?;
            Ok(node
                .path
                .iter()
                .position(|row| matches!(&*node.row(*row).block, Block::User(user) if &user.turn_id == turn)))
        })
    }

    fn earlier(&self, end: usize) -> LocalBoxFuture<'_, Result<EarlierRows, StoreError>> {
        Box::pin(async move { Ok(node(&self.stored.borrow(), &self.id)?.earlier(end)) })
    }

    fn prefix(&self, length: usize) -> LocalBoxFuture<'_, Result<Option<StoredPrefix>, StoreError>> {
        Box::pin(async move {
            let stored = self.stored.borrow();
            let node = node(&stored, &self.id)?;
            if length > node.path.len() {
                return Err(StoreError::Failed(format!("the path of node {} ends before {length}", self.id)));
            }
            Ok(length.checked_sub(1).map(|last| StoredPrefix {
                node: self.id.clone(),
                leaf: node.path[last],
                length,
            }))
        })
    }

    fn switch(&self, position: usize, version: u32, held_from: usize) -> LocalBoxFuture<'_, Result<Switched, StoreError>> {
        Box::pin(async move {
            let mut stored = self.stored.borrow_mut();
            let node = stored.nodes.get_mut(&self.id).ok_or_else(|| missing(&self.id))?;
            if position >= node.path.len() {
                return Err(StoreError::Failed(format!("the path of node {} ends before {position}", self.id)));
            }
            let following = Following::of_rows(&node.rows);
            let parent = node.shown_place(position, &following);
            let from = match parent {
                None => 0,
                Some(parent) => node
                    .path
                    .iter()
                    .position(|row| *row == parent)
                    .ok_or_else(|| StoreError::Failed(format!("the path of node {} leaves its place", self.id)))?
                    + 1,
            };
            let members = version_rows(following.of(parent), version)
                .ok_or_else(|| StoreError::Failed(format!("position {position} holds no version {version}")))?;
            // A row follows an older one, so one pass in order finds every
            // row below the version's; the latest of them is a leaf.
            let mut below: Vec<u64> = members;
            for (number, row) in (0u64..).zip(&node.rows) {
                if row.follows.is_some_and(|follows| below.contains(&follows)) && !below.contains(&number) {
                    below.push(number);
                }
            }
            let leaf = below.into_iter().max().expect("a version has a row");
            let mut tail = vec![leaf];
            while let Some(follows) = node.row(*tail.last().expect("the tail has a row")).follows
                && Some(follows) != parent
            {
                tail.push(follows);
            }
            tail.reverse();
            node.path.truncate(from);
            node.path.extend(tail);
            node.cut = None;
            let before = (from < held_from).then(|| {
                let start = node.last_of(Kind::CompactionBoundary, from + 1).unwrap_or(0);
                let mut versions = node.versions(start);
                versions.retain(|position, _| *position < from);
                SwitchedBefore {
                    start,
                    rows: Rows {
                        blocks: node.blocks(start..from),
                        versions,
                    },
                    earlier: node.earlier(start),
                }
            });
            Ok(Switched {
                from,
                blocks: node.blocks(from..node.path.len()),
                versions: node.versions(from),
                before,
            })
        })
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
            let stored = self.stored.borrow();
            let mut children: Vec<&StoredNode> = stored
                .nodes
                .values()
                .filter(|node| node.record.parent.as_ref() == Some(parent) && node.record.side.is_none())
                .collect();
            children.sort_by_key(|node| node.record.number);
            Ok(children
                .into_iter()
                .map(|node| node.record.clone())
                .collect())
        })
    }

    fn subagents(&self) -> LocalBoxFuture<'_, Result<Vec<StoredSubagent>, StoreError>> {
        Box::pin(async move {
            let stored = self.stored.borrow();
            let started_on_path = |record: &NodeRecord| {
                let (Some(call), Some(parent)) = (&record.started_by, &record.parent) else {
                    return true;
                };
                stored.nodes.get(parent).is_none_or(|parent| {
                    parent.path.iter().any(|row| parent.row(*row).block.id() == call)
                })
            };
            let mut subagents: Vec<StoredSubagent> = stored
                .nodes
                .values()
                .filter(|node| node.record.parent.is_some() && node.record.side.is_none())
                .map(|node| StoredSubagent {
                    record: node.record.clone(),
                    started_on_path: started_on_path(&node.record),
                })
                .collect();
            subagents.sort_by_key(|subagent| subagent.record.number);
            Ok(subagents)
        })
    }

    fn saved_versions(&self) -> LocalBoxFuture<'_, Result<crate::SavedVersions, StoreError>> {
        Box::pin(async move { Ok(crate::SavedVersions::default()) })
    }

    fn sides(&self) -> LocalBoxFuture<'_, Result<Vec<NodeRecord>, StoreError>> {
        Box::pin(async move {
            let stored = self.stored.borrow();
            let mut sides: Vec<NodeRecord> = stored
                .nodes
                .values()
                .filter(|node| node.record.side.is_some())
                .map(|node| node.record.clone())
                .collect();
            sides.sort_by_key(|record| (record.side.as_ref().map(|side| side.asked_at), record.id.clone()));
            Ok(sides)
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
                    rows: Vec::new(),
                    path: Vec::new(),
                    cut: None,
                },
            );
            apply_save(&mut stored, &id, initial)
        })
    }

    /// One store holds every tree a test makes, so the rows of `prefix`
    /// are this store's whatever `root` they belong to. It copies them, which
    /// no reader of the store can tell from a path that starts with them.
    fn create_node_after<'a>(
        &'a self,
        record: NodeRecord,
        _root: Option<&'a NodeId>,
        prefix: &'a StoredPrefix,
        initial: CheckpointUpdate,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let mut stored = self.stored.borrow_mut();
            if stored.nodes.contains_key(&record.id) {
                return Err(StoreError::Failed(format!("node {} already exists", record.id)));
            }
            let source = node(&stored, &prefix.node)?;
            let mut chain = vec![prefix.leaf];
            while let Some(follows) = source.row(*chain.last().expect("the chain has a row")).follows {
                chain.push(follows);
            }
            chain.reverse();
            if chain.len() != prefix.length {
                return Err(StoreError::Failed(format!("the prefix of node {} is not {} rows", prefix.node, prefix.length)));
            }
            let rows: Vec<StoredRow> = (0u64..)
                .zip(&chain)
                .map(|(number, row)| StoredRow {
                    block: source.row(*row).block.clone(),
                    follows: number.checked_sub(1),
                    replaces: None,
                })
                .collect();
            let id = record.id.clone();
            stored.nodes.insert(
                id.clone(),
                StoredNode {
                    record,
                    state: initial.state.clone(),
                    path: (0..u64::try_from(rows.len()).expect("a row number fits")).collect(),
                    rows,
                    cut: None,
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

    fn delete_node<'a>(&'a self, id: &'a NodeId) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let mut stored = self.stored.borrow_mut();
            // Only a side question's node, as the database's.
            if stored.nodes.get(id).is_none_or(|node| node.record.side.is_none()) {
                return Err(missing(id));
            }
            stored.nodes.remove(id);
            stored.running.remove(id);
            Ok(())
        })
    }

    fn reopen_node<'a>(
        &'a self,
        id: &'a NodeId,
        round: u64,
        started_at: Timestamp,
        message: Option<QueuedMessage>,
        reprofile: Option<Reprofile>,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let mut stored = self.stored.borrow_mut();
            let node = stored.nodes.get_mut(id).ok_or_else(|| missing(id))?;
            if let Some(reprofile) = reprofile {
                node.record.profile = Some(reprofile.name);
                node.record.model = Some(reprofile.model);
                node.record.instructions = reprofile.instructions;
                node.record.context_limit = reprofile.context_limit;
                node.record.can_spawn_subagents = reprofile.can_spawn_subagents;
            }
            node.record.closed = None;
            node.record.delivered = false;
            node.record.round = round;
            node.record.started_at = started_at;
            if let Some(message) = message {
                node.state.queue = vec![message];
            }
            Ok(())
        })
    }

    fn set_cwd<'a>(
        &'a self,
        id: &'a NodeId,
        cwd: &'a ChildDirectory,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        Box::pin(async move {
            let mut stored = self.stored.borrow_mut();
            let node = stored.nodes.get_mut(id).ok_or_else(|| missing(id))?;
            node.record.cwd = Some(cwd.clone());
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

    fn next_number(&self, sequence: Sequence) -> LocalBoxFuture<'_, Result<u64, StoreError>> {
        Box::pin(async move {
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
    ) -> LocalBoxFuture<'a, Result<Option<StoredCommand>, StoreError>> {
        Box::pin(async move { Ok(self.stored.borrow().outputs.get(command).cloned()) })
    }

    fn command_end<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Result<Option<CommandEnd>, StoreError>> {
        Box::pin(async move {
            Ok(self.stored.borrow().outputs.get(command).map(|stored| stored.end.clone()))
        })
    }

    fn running_commands<'a>(
        &'a self,
        node: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<Vec<RunningCommand>, StoreError>> {
        let stored = self.stored.borrow();
        let running = stored.running.get(node).cloned().unwrap_or_default();
        Box::pin(async move { Ok(running) })
    }

    fn running_nodes(&self) -> LocalBoxFuture<'_, Result<BTreeSet<NodeId>, StoreError>> {
        let running = self
            .stored
            .borrow()
            .running
            .iter()
            .filter(|(_, commands)| !commands.is_empty())
            .map(|(node, _)| node.clone())
            .collect();
        Box::pin(async move { Ok(running) })
    }

    fn record_place<'a>(
        &'a self,
        command: &'a CommandId,
        node: &'a NodeId,
        seen: demi_host_interface::Seen,
    ) -> LocalBoxFuture<'a, Result<(), StoreError>> {
        let mut stored = self.stored.borrow_mut();
        let runs = stored.running.values().flatten().any(|running| running.command == *command);
        if runs {
            stored.places.insert((command.clone(), node.clone()), seen);
            for running in stored.running.get_mut(node).into_iter().flatten() {
                if running.command == *command {
                    running.place = seen;
                }
            }
        }
        Box::pin(async { Ok(()) })
    }

    fn place<'a>(
        &'a self,
        command: &'a CommandId,
        node: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<Option<demi_host_interface::Seen>, StoreError>> {
        let place = self.stored.borrow().places.get(&(command.clone(), node.clone())).copied();
        Box::pin(async move { Ok(place) })
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
