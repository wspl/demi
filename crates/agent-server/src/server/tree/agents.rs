//! The conversation's agents as its metadata lists them, a collection the tree
//! owns and keeps in sync.
//!
//! Every node, the root, each child live or closed and each side question, has
//! one entry. Whoever changes a fact an entry shows changes the entry here, in
//! the step it changes, and the change goes to every attached connection as
//! that one entry, `agent`, at the collection's next version. The collection
//! itself is read by the conversation's route, whole at its version, and a
//! page that follows it at a version it holds is sent the entries changed
//! since ([`Agents::since`]).
//!
//! ```text
//!  session events ──┐
//!  child start, close, cwd, path ──┼──▶ entry changed? ──▶ revision + 1
//!  ledger row, side card (product) ┘                       `agent` frame
//! ```
//!
//! Each agent's cache chart, which the product counts as its turn calls land
//! (`crates/backend-database/src/usage.rs`), is kept here beside its cost but
//! is no part of its entry: it goes whole, as `cache_chart`, to the
//! connections that follow the agent's node as it changes, and the page
//! reads it by a route when its card opens.
//!
//! The epoch is new at each open of the tree, so a page that held the agents
//! before a restart of the backend reads them again.
//!
//! A step of a session, one change of it, changes each entry once: what it
//! changes of one entry, such as its starting message and its phase, goes as
//! one `agent` frame at one revision when the step ends ([`Agents::step`]),
//! so no page sees an entry half changed.

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
};

use demi_agent_store::{NodeRecord, Placement, SideOf};
use demi_conversation_socket_protocol::{
    AgentCost, AgentEntry, AgentRole, CacheChart, JobModel, ServerFrame, SideQuestion, Version,
};
use demi_shared_types::{NodeId, SessionPhase};

use super::FrameSink;

/// A side question's card as the product keeps it: its title, null until
/// the small model wrote one, and whether it is folded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SideCard {
    pub title: Option<String>,
    pub folded: bool,
}

/// What the product keeps of a conversation's agents that their entries
/// show: the cost of each agent by its number, the root's 0, and each side
/// question's card. The agents' cache charts, which only a live tree sends,
/// are read apart (`AgentsSource::charts`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct KeptAgents {
    pub costs: HashMap<u64, AgentCost>,
    pub cards: HashMap<NodeId, SideCard>,
}

/// One entry and the revision it last changed at.
struct Kept {
    revision: u64,
    entry: AgentEntry,
}

/// The tree's agents.
pub(crate) struct Agents {
    epoch: String,
    revision: Cell<u64>,
    entries: RefCell<HashMap<NodeId, Kept>>,
    /// What the product told of the agents, kept for the entries still to
    /// come and for those it changes.
    costs: RefCell<HashMap<u64, AgentCost>>,
    charts: RefCell<HashMap<u64, CacheChart>>,
    cards: RefCell<HashMap<NodeId, SideCard>>,
    /// The agents whose cost a ledger row announced since the tree opened,
    /// which what the product kept before that leaves as they are.
    announced: RefCell<HashSet<u64>>,
    /// The agents whose chart a turn call's row announced since the tree
    /// opened, apart from `announced`: a row of no turn call announces a
    /// cost but leaves the chart the product kept.
    charted: RefCell<HashSet<u64>>,
    /// Where the conversation worked when last read, which a child's entry
    /// names its directory by (`crates/agent-server/src/node/mod.rs`).
    placement: RefCell<Option<Placement>>,
    /// The entries the step under way changed, announced when it ends.
    staged: RefCell<Vec<AgentEntry>>,
    /// How deep the steps under way nest.
    depth: Cell<usize>,
    sink: Rc<FrameSink>,
}

impl Agents {
    pub(crate) fn new(epoch: String, sink: Rc<FrameSink>) -> Rc<Self> {
        Rc::new(Self {
            epoch,
            revision: Cell::new(0),
            entries: RefCell::default(),
            costs: RefCell::default(),
            charts: RefCell::default(),
            cards: RefCell::default(),
            announced: RefCell::default(),
            charted: RefCell::default(),
            placement: RefCell::new(None),
            staged: RefCell::default(),
            depth: Cell::new(0),
            sink,
        })
    }

    pub(crate) fn version(&self) -> Version {
        Version {
            epoch: self.epoch.clone(),
            revision: self.revision.get(),
        }
    }

    /// Takes `entry` as its agent's, with the cost and card the product
    /// told; a change is announced.
    pub(crate) fn put(&self, mut entry: AgentEntry) {
        if let Some(number) = number_of(&entry.role) {
            entry.cost = self.costs.borrow().get(&number).copied().unwrap_or_default();
        }
        if let AgentRole::Side(side) = &mut entry.role {
            let card = self.cards.borrow().get(&entry.id).cloned().unwrap_or_default();
            side.title = card.title;
            side.folded = card.folded;
        }
        if self.depth.get() > 0 {
            let mut staged = self.staged.borrow_mut();
            match staged.iter_mut().find(|each| each.id == entry.id) {
                Some(each) => *each = entry,
                None => staged.push(entry),
            }
            return;
        }
        self.announce(entry);
    }

    /// Runs `deliver`, the delivery of one step of a session, and announces
    /// each entry it changed once, when the step ends, among the step's frames.
    pub(crate) fn step(&self, deliver: impl FnOnce()) {
        self.sink.step(|| {
            self.depth.set(self.depth.get() + 1);
            deliver();
            self.depth.set(self.depth.get() - 1);
            if self.depth.get() == 0 {
                let staged = std::mem::take(&mut *self.staged.borrow_mut());
                for entry in staged {
                    self.announce(entry);
                }
            }
        });
    }

    /// Keeps `entry` at the next revision and sends it, unless it is the entry
    /// held already.
    fn announce(&self, entry: AgentEntry) {
        let unchanged = self
            .entries
            .borrow()
            .get(&entry.id)
            .is_some_and(|kept| kept.entry == entry);
        if unchanged {
            return;
        }
        let revision = self.revision.get() + 1;
        self.revision.set(revision);
        self.entries.borrow_mut().insert(
            entry.id.clone(),
            Kept {
                revision,
                entry: entry.clone(),
            },
        );
        self.sink.emit_to_agents(ServerFrame::Agent {
            version: Version {
                epoch: self.epoch.clone(),
                revision,
            },
            agent: entry,
        });
    }

    /// The agent `id` was deleted: its entry stays as a tombstone, which the
    /// `agent` frame announces at the next revision and the changes since a
    /// revision a page holds list, so every page that holds it drops it. The
    /// whole `agents` frame lists no tombstone.
    pub(crate) fn delete(&self, id: &NodeId) {
        self.cards.borrow_mut().remove(id);
        self.change(id, |entry| entry.deleted = true);
    }

    /// Changes the entry of the agent `id` with `change`; nothing for an
    /// agent the collection does not hold yet.
    pub(crate) fn change(&self, id: &NodeId, change: impl FnOnce(&mut AgentEntry)) {
        let Some(mut entry) = self.entry(id) else {
            return;
        };
        change(&mut entry);
        self.put(entry);
    }

    /// The entry of the agent `id` as the step under way leaves it.
    pub(crate) fn entry(&self, id: &NodeId) -> Option<AgentEntry> {
        let staged = self.staged.borrow().iter().find(|each| &each.id == id).cloned();
        staged.or_else(|| self.entries.borrow().get(id).map(|kept| kept.entry.clone()))
    }

    /// What the product kept when the tree opened: a cost a row announced
    /// since stays.
    pub(crate) fn adopt(&self, kept: KeptAgents) {
        {
            let announced = self.announced.borrow();
            let mut costs = self.costs.borrow_mut();
            for (number, cost) in kept.costs {
                if !announced.contains(&number) {
                    costs.insert(number, cost);
                }
            }
            self.cards.borrow_mut().extend(kept.cards);
        }
        self.refresh_all();
    }

    /// The charts the product kept when the tree opened: a chart a turn
    /// call announced since stays.
    pub(crate) fn adopt_charts(&self, kept: HashMap<u64, CacheChart>) {
        let adopted: Vec<u64> = {
            let charted = self.charted.borrow();
            let mut charts = self.charts.borrow_mut();
            kept.into_iter()
                .filter(|(number, _)| !charted.contains(number))
                .map(|(number, chart)| {
                    charts.insert(number, chart);
                    number
                })
                .collect()
        };
        for number in adopted {
            self.announce_chart(number);
        }
    }

    /// The agent `number` costs `cost` now, as its latest ledger row made it,
    /// and its cache chart is `chart`, for a row of a turn call. The entry
    /// and the chart go as one step, so a page never shows the row's cost
    /// with the chart before it.
    pub(crate) fn cost_changed(&self, number: u64, cost: AgentCost, chart: Option<CacheChart>) {
        self.step(|| {
            self.announced.borrow_mut().insert(number);
            self.costs.borrow_mut().insert(number, cost);
            for id in self.ids_of(number) {
                self.change(&id, |_| {});
            }
            if let Some(chart) = chart {
                self.charted.borrow_mut().insert(number);
                self.charts.borrow_mut().insert(number, chart);
                self.announce_chart(number);
            }
        });
    }

    /// The nodes of the agent `number`.
    fn ids_of(&self, number: u64) -> Vec<NodeId> {
        self.entries
            .borrow()
            .values()
            .filter(|kept| number_of(&kept.entry.role) == Some(number))
            .map(|kept| kept.entry.id.clone())
            .collect()
    }

    /// Sends the agent `number`'s chart to the connections that follow its
    /// node.
    fn announce_chart(&self, number: u64) {
        for id in self.ids_of(number) {
            if let Some(chart) = self.chart(&id) {
                self.sink.emit_to(&id, ServerFrame::CacheChart { node: id.clone(), chart });
            }
        }
    }

    /// The cache chart of the node `id`, an empty chart for an agent that
    /// made no turn call; none for a side question or a node the collection
    /// does not hold.
    pub(crate) fn chart(&self, id: &NodeId) -> Option<CacheChart> {
        let number = number_of(&self.entries.borrow().get(id)?.entry.role)?;
        Some(self.charts.borrow().get(&number).cloned().unwrap_or_default())
    }

    /// The side question `id`'s card is `card` now.
    pub(crate) fn card_changed(&self, id: &NodeId, card: SideCard) {
        self.cards.borrow_mut().insert(id.clone(), card);
        self.change(id, |_| {});
    }

    /// Takes what the product kept into every entry.
    fn refresh_all(&self) {
        let ids: Vec<NodeId> = self.entries.borrow().keys().cloned().collect();
        for id in ids {
            self.change(&id, |_| {});
        }
    }

    pub(crate) fn placement(&self) -> Option<Placement> {
        self.placement.borrow().clone()
    }

    /// Where the conversation works now; answers whether it moved.
    pub(crate) fn set_placement(&self, placement: Option<Placement>) -> bool {
        let moved = *self.placement.borrow() != placement;
        if moved {
            self.placement.replace(placement);
        }
        moved
    }

    /// The entries changed after the revision `since`, with those that wait
    /// for their model, whose wait the page counts from the frame, as the
    /// `agents` frame the follow of a page that holds the agents at `since`
    /// is answered with. `waits` tells how long each live agent has waited
    /// now.
    pub(crate) fn since(&self, since: u64, waits: impl Fn(&NodeId) -> Option<u64>) -> ServerFrame {
        let mut agents: Vec<AgentEntry> = self
            .entries
            .borrow()
            .values()
            .filter_map(|kept| {
                let mut entry = kept.entry.clone();
                entry.waited_ms = entry.waited_ms.and_then(|_| waits(&entry.id));
                (kept.revision > since || entry.waited_ms.is_some()).then_some(entry)
            })
            .collect();
        agents.sort_by_key(rank);
        ServerFrame::Agents {
            version: self.version(),
            since: Some(since),
            agents,
        }
    }

    /// Every entry but a tombstone's, in the order the page lists them: the
    /// root, the subagents by number, then the side questions as they were asked. A live
    /// agent's wait is how long it has waited now, `waits` tells, which the page
    /// counts from the answer.
    pub(crate) fn all(&self, waits: impl Fn(&NodeId) -> Option<u64>) -> Vec<AgentEntry> {
        let mut agents: Vec<AgentEntry> = self
            .entries
            .borrow()
            .values()
            .filter(|kept| !kept.entry.deleted)
            .map(|kept| {
                let mut entry = kept.entry.clone();
                entry.waited_ms = entry.waited_ms.and_then(|_| waits(&entry.id));
                entry
            })
            .collect();
        agents.sort_by_key(rank);
        agents
    }
}

/// The ledger's number of an agent: the root's 0 and a subagent's own. A
/// side question's rows carry the root's number, so its entry carries no
/// cost of its own.
fn number_of(role: &AgentRole) -> Option<u64> {
    match role {
        AgentRole::Root => Some(0),
        AgentRole::Subagent(subagent) => Some(subagent.number),
        AgentRole::Side(_) => None,
    }
}

/// The root first, then the subagents by number, then the side questions as
/// they were asked.
fn rank(entry: &AgentEntry) -> (u8, u64, i64) {
    match &entry.role {
        AgentRole::Root => (0, 0, 0),
        AgentRole::Subagent(subagent) => (1, subagent.number, 0),
        AgentRole::Side(side) => (2, 0, side.asked_at.as_millisecond()),
    }
}

/// The entry of an agent that is not live: idle, waiting for nothing.
pub(crate) fn resting(id: NodeId, role: AgentRole) -> AgentEntry {
    AgentEntry {
        id,
        role,
        phase: SessionPhase::Idle,
        waited_ms: None,
        pending_calls: Vec::new(),
        context: None,
        cost: AgentCost::default(),
        queue: Vec::new(),
        pending_steers: Vec::new(),
        starting: None,
        deleted: false,
    }
}

/// The role of the child `record`, whose round runs on `runs_on`, while the
/// conversation works at `placement`.
pub(crate) fn subagent_role(record: &NodeRecord, runs_on: JobModel, placement: Option<&Placement>, on_path: bool) -> AgentRole {
    AgentRole::Subagent(record.child(runs_on, placement, on_path).expect("a subagent's record names its parent"))
}

/// The role of the side question that asked `of`; its card is the product's.
pub(crate) fn side_role(of: SideOf) -> AgentRole {
    AgentRole::Side(SideQuestion {
        block_id: of.block,
        start: of.start,
        end: of.end,
        quote: of.quote,
        picture: of.picture,
        question: of.question,
        title: None,
        folded: false,
        asked_at: of.asked_at,
    })
}
