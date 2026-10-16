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
    B64Bytes, BlobRef, Block, BlockId, CommandEnd, CommandId, FileExtension, Model, ModelSelection, NodeId,
    QueuedMessage, Sequence, Timestamp, UserContentBlock,
};
use futures_util::future::LocalBoxFuture;

use demi_conversation_socket_protocol::Versions;

use crate::{
    AgentTreeStore, Checkpoint, CheckpointState, CheckpointUpdate, ChildDirectory, NodeClose, NodeRecord,
    RunningCommand, SessionStore, StoreError, StoredCommand, StoredSubagent, Switched,
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

/// A PNG of `width` × `height` px whose colors change smoothly with a little
/// noise in them, as a drawn illustration's: it compresses poorly as PNG and
/// well as JPEG. Each pixel's alpha is `alpha(x, y)`.
pub fn illustration(width: u32, height: u32, alpha: impl Fn(u32, u32) -> u8) -> B64Bytes {
    use image::{DynamicImage, Rgba, RgbaImage, codecs::png::PngEncoder};

    let mut noise = noise(7);
    let pixels = RgbaImage::from_fn(width, height, |x, y| {
        let r = u8::try_from(x * 255 / width).expect("a share of 255 fits");
        let g = u8::try_from(y * 255 / height).expect("a share of 255 fits");
        let n = noise() >> 4;
        Rgba([r.saturating_add(n), g.saturating_add(n), 128 + n, alpha(x, y)])
    });
    let mut bytes = Vec::new();
    DynamicImage::ImageRgba8(pixels)
        .write_with_encoder(PngEncoder::new(&mut bytes))
        .expect("a PNG encodes into memory");
    B64Bytes::from(bytes)
}

/// A PNG of `width` × `height` px speckled at random with the eight colors
/// whose channels are all full or empty, about half a byte a pixel as PNG:
/// as a JPEG, which keeps every speck's edges, it would be larger.
pub fn speckled(width: u32, height: u32) -> B64Bytes {
    use image::{
        DynamicImage, Rgb, RgbImage,
        codecs::png::{CompressionType, FilterType, PngEncoder},
    };

    let mut noise = noise(11);
    let mut channel = move || if noise() < 128 { 0 } else { 255 };
    let pixels = RgbImage::from_fn(width, height, |_, _| Rgb([channel(), channel(), channel()]));
    let mut bytes = Vec::new();
    let encoder = PngEncoder::new_with_quality(&mut bytes, CompressionType::Default, FilterType::NoFilter);
    DynamicImage::ImageRgb8(pixels)
        .write_with_encoder(encoder)
        .expect("a PNG encodes into memory");
    B64Bytes::from(bytes)
}

/// Bytes that look random and are the same each run, from `seed`.
fn noise(seed: u32) -> impl FnMut() -> u8 {
    let mut state = seed;
    move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        state.to_be_bytes()[0]
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

/// One row of a node's tree.
#[derive(Debug, Clone)]
struct StoredRow {
    block: Block,
    follows: Option<u64>,
    replaces: Option<u64>,
}

impl StoredNode {
    fn row(&self, row: u64) -> &StoredRow {
        &self.rows[usize::try_from(row).expect("a row number fits")]
    }

    /// The rows that follow `parent`, with the rows they replace.
    fn siblings(&self, parent: Option<u64>) -> Vec<Sibling> {
        (0u64..)
            .zip(&self.rows)
            .filter(|(_, row)| row.follows == parent)
            .map(|(number, row)| Sibling {
                row: number,
                replaces: row.replaces,
            })
            .collect()
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
    fn shown_place(&self, position: usize) -> Option<u64> {
        let own = position.checked_sub(1).map(|before| self.path[before]);
        let row = self.path[position];
        if versions_of(&self.siblings(own), row).is_some() {
            return own;
        }
        self.row(self.carried(row)).follows
    }

    /// The versions at each position of the path from `from` on: a place's
    /// at the block that stands there, or at the block a compaction carried
    /// from there, which the boundary then stands in for without showing
    /// them (`message-editing.md` § Versions).
    fn versions(&self, from: usize) -> BTreeMap<usize, Versions> {
        let mut shown: BTreeMap<Option<u64>, (usize, Versions)> = BTreeMap::new();
        for position in 0..self.path.len() {
            let parent = self.shown_place(position);
            let row = self.carried(self.path[position]);
            if let Some(versions) = versions_of(&self.siblings(parent), row) {
                shown.insert(parent, (position, versions));
            }
        }
        shown.into_values().filter(|(position, _)| *position >= from).collect()
    }
}

#[derive(Debug, Default, Clone)]
struct Stored {
    nodes: BTreeMap<NodeId, StoredNode>,
    saves: Vec<(NodeId, CheckpointUpdate)>,
    /// The next number of each sequence that gave one out.
    sequences: BTreeMap<Sequence, u64>,
    failing_saves: usize,
    /// The nodes whose checkpoint reads as corrupt.
    unreadable: Vec<NodeId>,
    save_hold: Option<Rc<Hold>>,
    number_hold: Option<Rc<Hold>>,
    /// The holds on reads of a node's children, by the node.
    children_holds: BTreeMap<NodeId, Rc<Hold>>,
    end_hold: Option<Rc<Hold>>,
    /// What the product's keeper stored of each ended command's output.
    outputs: BTreeMap<CommandId, StoredCommand>,
    /// The commands each node ran that are recorded running.
    running: BTreeMap<NodeId, Vec<RunningCommand>>,
    /// How far each node looked in each running command's output.
    places: BTreeMap<(CommandId, NodeId), demi_host_interface::Seen>,
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
    pub fn keep_output(&self, command: CommandId, stored: StoredCommand) {
        self.stored.borrow_mut().outputs.insert(command, stored);
    }

    /// Records `running` as a command `node` ran that runs, as the
    /// product's keeper does when its job starts.
    pub fn record_running(&self, node: NodeId, running: RunningCommand) {
        self.stored.borrow_mut().running.entry(node).or_default().push(running);
    }

    /// Makes the checkpoint of `id` read as corrupt from now on, as a
    /// stored state that no longer decodes does.
    pub fn corrupt(&self, id: &NodeId) {
        self.stored.borrow_mut().unreadable.push(id.clone());
    }

    /// Every save so far, in order, with the node it was for.
    pub fn saves(&self) -> Vec<(NodeId, CheckpointUpdate)> {
        self.stored.borrow().saves.clone()
    }

    /// Every row the node's tree holds, on its path or not, in the order
    /// they were written.
    pub fn rows(&self, id: &NodeId) -> Vec<Block> {
        self.stored
            .borrow()
            .nodes
            .get(id)
            .map(|node| node.rows.iter().map(|row| row.block.clone()).collect())
            .unwrap_or_default()
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

    /// Holds every read of how a command ended from now on until the gate
    /// is released.
    pub fn hold_command_ends(&self) -> StoreGate {
        let hold = Rc::new(Hold::default());
        self.stored.borrow_mut().end_hold = Some(hold.clone());
        StoreGate(hold)
    }
}

fn missing(id: &NodeId) -> StoreError {
    StoreError::Failed(format!("no node {id}"))
}

fn load(stored: &Stored, id: &NodeId) -> Result<Option<Checkpoint>, StoreError> {
    if stored.unreadable.contains(id) {
        return Err(StoreError::Corrupt(format!("the checkpoint of node {id} does not decode")));
    }
    let Some(node) = stored.nodes.get(id) else {
        return Ok(None);
    };
    Ok(Some(Checkpoint {
        state: node.state.clone(),
        transcript: node.path.iter().map(|row| node.row(*row).block.clone()).collect(),
        versions: node.versions(0),
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
            node.rows[row].block = block.clone();
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
            block: block.clone(),
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

    fn switch(&self, position: usize, version: u32) -> LocalBoxFuture<'_, Result<Switched, StoreError>> {
        Box::pin(async move {
            let mut stored = self.stored.borrow_mut();
            let node = stored.nodes.get_mut(&self.id).ok_or_else(|| missing(&self.id))?;
            if position >= node.path.len() {
                return Err(StoreError::Failed(format!("the path of node {} ends before {position}", self.id)));
            }
            let parent = node.shown_place(position);
            let from = match parent {
                None => 0,
                Some(parent) => node
                    .path
                    .iter()
                    .position(|row| *row == parent)
                    .ok_or_else(|| StoreError::Failed(format!("the path of node {} leaves its place", self.id)))?
                    + 1,
            };
            let members = version_rows(&node.siblings(parent), version)
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
            Ok(Switched {
                from,
                blocks: node.path[from..].iter().map(|row| node.row(*row).block.clone()).collect(),
                versions: node.versions(from),
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
                .filter(|node| node.record.parent.is_some())
                .map(|node| StoredSubagent {
                    record: node.record.clone(),
                    model: node.state.model.clone(),
                    started_on_path: started_on_path(&node.record),
                })
                .collect();
            subagents.sort_by_key(|subagent| subagent.record.number);
            Ok(subagents)
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
    ) -> LocalBoxFuture<'a, Result<Option<StoredCommand>, StoreError>> {
        Box::pin(async move { Ok(self.stored.borrow().outputs.get(command).cloned()) })
    }

    fn command_end<'a>(
        &'a self,
        command: &'a CommandId,
    ) -> LocalBoxFuture<'a, Result<Option<CommandEnd>, StoreError>> {
        Box::pin(async move {
            let hold = self.stored.borrow().end_hold.clone();
            if let Some(hold) = hold {
                hold.pass().await;
            }
            Ok(self.stored.borrow().outputs.get(command).map(|stored| stored.end.clone()))
        })
    }

    fn running_commands<'a>(
        &'a self,
        node: &'a NodeId,
    ) -> LocalBoxFuture<'a, Result<Vec<RunningCommand>, StoreError>> {
        let running = self.stored.borrow().running.get(node).cloned().unwrap_or_default();
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
        AgentTreeStore, CheckpointState, CheckpointUpdate, ChildDirectory, ClosePhase, NodeClose,
        NodeRecord, PendingAgentInput, Placement,
    };

    fn id(value: &str) -> NodeId {
        NodeId::try_from(value).expect("a test node id is not empty")
    }

    /// A node in its first round, agent `number` of the conversation; a
    /// child works in `/work`.
    fn record(node: &str, parent: Option<&str>, number: u64) -> NodeRecord {
        NodeRecord {
            id: id(node),
            number,
            parent: parent.map(id),
            description: String::new(),
            profile: None,
            instructions: None,
            context_limit: None,
            round: 1,
            started_at: Timestamp::UNIX_EPOCH,
            can_spawn_subagents: true,
            closed: None,
            delivered: false,
            forked_from: None,
            started_by: None,
            cwd: parent.map(|_| directory("/work")),
        }
    }

    /// `cwd`, set while the conversation worked in `/work` on `laptop`.
    fn directory(cwd: &str) -> ChildDirectory {
        ChildDirectory {
            cwd: cwd.to_owned(),
            under: Placement {
                device: Some("laptop".to_owned()),
                directory: "/work".to_owned(),
            },
        }
    }

    fn update(queue: Vec<QueuedMessage>, blocks: Vec<Block>) -> CheckpointUpdate {
        CheckpointUpdate {
            state: CheckpointState {
                phase: SessionPhase::Idle,
                queue,
                agent_inputs: Vec::new(),
                reports: Vec::new(),
                next_turn_reports: Vec::new(),
                intervals: Vec::new(),
                model: test_model(),
                edits: Vec::new(),
                account: None,
            },
            block_count: blocks.len(),
            changed_blocks: blocks.into_iter().enumerate().collect(),
            moved_from: Some(0),
            version: false,
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
            sender: Some(Sender {
                id: id(child),
                number: 1,
                description: child.into(),
                round,
            }),
            recipient_id: id("root"),
            timestamp: Timestamp::UNIX_EPOCH,
            content: format!("{child} done"),
            event: AgentMessageEvent::Completion {
                outcome: CompletionOutcome::Completed,
            },
            origin: None,
        };
        Block::AgentMessage(AgentMessageBlock {
            run: None,
            entries: Vec::new(),
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

    /// A block of the agent's text `text`, its id the text.
    fn said(text: &str) -> Block {
        Block::Text(demi_shared_types::TextBlock {
            id: text.try_into().expect("a test block id is not empty"),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            text: text.to_owned(),
            forkable: true,
            entries: Vec::new(),
        })
    }

    /// A save of the path's blocks `blocks` from `from` on, of `length`
    /// blocks, as a rewrite from `from` that `version` makes a version.
    fn rewrite(from: usize, blocks: Vec<Block>, length: usize, version: bool) -> CheckpointUpdate {
        CheckpointUpdate {
            changed_blocks: (from..).zip(blocks).collect(),
            block_count: length,
            moved_from: Some(from),
            version,
            ..update(Vec::new(), Vec::new())
        }
    }

    fn versions(index: u32, count: u32) -> demi_conversation_socket_protocol::Versions {
        demi_conversation_socket_protocol::Versions { index, count }
    }

    /// `message-editing.md` § Versions: an edit adds a version beside the
    /// rows it leaves, which stay as written; the session's own rewrite
    /// stands in for what it replaces and is no version; Regenerate's cut
    /// makes the next row a version of its own; a switch makes the latest
    /// leaf of a version current, and the load restores that path.
    pub async fn a_rewrite_keeps_its_rows_and_a_switch_brings_them_back(store: &dyn AgentTreeStore) {
        create(store, record("root", None, 0), update(Vec::new(), Vec::new())).await;
        let root = store.session_store(&id("root"));
        let load = || async {
            let checkpoint = root.load().await.expect("the store reads").expect("the node has one");
            (checkpoint.transcript, checkpoint.versions)
        };
        let first = vec![said("u1"), said("a1"), said("u2"), said("a2")];
        root.save(update(Vec::new(), first.clone())).await.expect("the save commits");

        // An edit of the second message.
        root.save(rewrite(2, vec![said("u2-edited"), said("a2-edited")], 4, true))
            .await
            .expect("the edit commits");
        let (blocks, held) = load().await;
        assert_eq!(blocks, [said("u1"), said("a1"), said("u2-edited"), said("a2-edited")]);
        assert_eq!(held.into_iter().collect::<Vec<_>>(), [(2, versions(2, 2))]);
        // A retry of its answer is no version of its own.
        root.save(rewrite(3, vec![said("a2-retried")], 4, false))
            .await
            .expect("the retry commits");
        let (_, held) = load().await;
        assert_eq!(held.into_iter().collect::<Vec<_>>(), [(2, versions(2, 2))]);

        // Back to the first version: its rows as they were written.
        let switched = root.switch(2, 1).await.expect("the switch commits");
        assert_eq!(switched.blocks, [said("u2"), said("a2")]);
        assert_eq!(switched.versions.into_iter().collect::<Vec<_>>(), [(2, versions(1, 2))]);
        assert_eq!(load().await.0, first);
        // And to the second again: where that branch ended, the retry.
        let switched = root.switch(2, 2).await.expect("the switch commits");
        assert_eq!(switched.blocks, [said("u2-edited"), said("a2-retried")]);
        assert!(root.switch(2, 3).await.is_err(), "a version its place lacks");
        assert!(root.switch(4, 1).await.is_err(), "a place past the path");

        // Regenerate cuts after the message; the answer that follows in a
        // later save is a version of its own, and the retry's row and the
        // row it replaced are one version.
        root.save(rewrite(3, Vec::new(), 3, true)).await.expect("the cut commits");
        root.save(CheckpointUpdate {
            moved_from: Some(3),
            ..update(Vec::new(), vec![said("u1"), said("a1"), said("u2-edited"), said("a2-regenerated")])
        })
        .await
        .expect("the answer commits");
        let (blocks, held) = load().await;
        assert_eq!(blocks[3], said("a2-regenerated"));
        assert_eq!(
            held.into_iter().collect::<Vec<_>>(),
            [(2, versions(2, 2)), (3, versions(2, 2))]
        );
    }

    /// `message-editing.md` § Versions: a subagent tells whether the call
    /// that started it is on its parent's current path; one no call
    /// started is on every path. Builds the tree the store's own reading
    /// of whole subtrees is checked on: `root` holds `a1`, which started
    /// `kept`; `gone` was started by a block the root's path no longer
    /// holds, and `under` by `g1`, which `gone`'s path holds.
    pub async fn a_subagent_tells_whether_its_start_is_on_its_parents_path(store: &dyn AgentTreeStore) {
        create(store, record("root", None, 0), update(Vec::new(), vec![said("u1"), said("a1")])).await;
        for (node, parent, number, start, blocks) in [
            ("kept", "root", 1, Some("a1"), vec![said("k1")]),
            ("gone", "root", 2, Some("a0"), vec![said("g1")]),
            ("under", "gone", 3, Some("g1"), vec![said("n1")]),
            ("unknown", "root", 4, None, Vec::new()),
        ] {
            let child = NodeRecord {
                started_by: start.map(|block| block.try_into().expect("a test block id is not empty")),
                ..record(node, Some(parent), number)
            };
            create(store, child, update(Vec::new(), blocks)).await;
        }
        let started: Vec<(u64, bool)> = store
            .subagents()
            .await
            .expect("the store reads")
            .into_iter()
            .map(|subagent| (subagent.record.number, subagent.started_on_path))
            .collect();
        assert_eq!(started, [(1, true), (2, false), (3, true), (4, true)]);
    }

    /// `message-editing.md` § Versions: a message that compaction carries
    /// past its boundary keeps its versions, which its new position shows
    /// and the boundary does not, and switching there brings back the
    /// branch before the compaction.
    pub async fn a_compaction_keeps_the_versions_of_what_it_carries(store: &dyn AgentTreeStore) {
        create(store, record("root", None, 0), update(Vec::new(), Vec::new())).await;
        let root = store.session_store(&id("root"));
        let load = || async {
            let checkpoint = root.load().await.expect("the store reads").expect("the node has one");
            (checkpoint.transcript, checkpoint.versions.into_iter().collect::<Vec<_>>())
        };
        let first = vec![said("u1"), said("a1"), said("u2"), said("a2")];
        root.save(update(Vec::new(), first.clone())).await.expect("the save commits");
        root.save(rewrite(2, vec![said("u2-edited"), said("a2-edited")], 4, true))
            .await
            .expect("the edit commits");

        // The boundary goes in before the edited message, which it keeps.
        let carried = vec![said("u1"), said("a1"), said("boundary"), said("u2-edited"), said("a2-edited")];
        root.save(CheckpointUpdate {
            moved_from: Some(2),
            changed_blocks: (2..).zip(carried[2..].iter().cloned()).collect(),
            ..update(Vec::new(), carried.clone())
        })
        .await
        .expect("the compaction commits");
        assert_eq!(load().await, (carried.clone(), vec![(3, versions(2, 2))]));

        // Its first version is the branch before the compaction.
        let switched = root.switch(3, 1).await.expect("the switch commits");
        assert_eq!(switched.from, 2);
        assert_eq!(switched.blocks, [said("u2"), said("a2")]);
        assert_eq!(load().await, (first, vec![(2, versions(1, 2))]));
        let switched = root.switch(2, 2).await.expect("the switch commits");
        assert_eq!(switched.blocks, carried[2..]);
        assert_eq!(load().await, (carried, vec![(3, versions(2, 2))]));
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
        // What a child was spawned with stays in its record.
        let spawned = NodeRecord {
            profile: Some("explore".into()),
            instructions: Some("explore prompt".into()),
            context_limit: Some(200_000),
            can_spawn_subagents: false,
            ..record("child", Some("root"), 1)
        };
        create(
            store,
            spawned.clone(),
            update(vec![message("m1")], Vec::new()),
        )
        .await;

        assert_eq!(stored(store, "child").await, Some(spawned));
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
            entries: Vec::new(),
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
        // A child's changed directory is its record's from then on.
        store
            .set_cwd(&id("child"), &directory("/work/app"))
            .await
            .expect("the change commits");
        let moved = stored(store, "child").await.expect("the child is stored");
        assert_eq!(moved.cwd, Some(directory("/work/app")));
        assert_eq!(stored(store, "root").await.expect("the root is stored").cwd, None);
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

    pub async fn reopen_makes_a_closed_node_live_with_its_message(
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
            Sequence::Tab,
            Sequence::Agent,
            Sequence::Command,
            Sequence::Tab,
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
                (Sequence::Tab, 1),
                (Sequence::Agent, 1),
                (Sequence::Command, 3),
                (Sequence::Tab, 2),
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
