//! The conversation's agents as its metadata lists them, a collection the tree
//! owns and keeps in sync (`docs/decisions/0009-agent-sessions-and-sync.md`).
//!
//! Every node, the root, each child live or closed and each side question, has
//! one entry. Whoever changes a fact an entry shows changes the entry here, in
//! the step it changes, and the change goes to every attached connection as
//! that one entry, `agent`, with the collection's next revision. An open is
//! answered with the entries changed since the revision the page holds, or
//! all of them when it holds another epoch.
//!
//! ```text
//!  session events ──┐
//!  child start, close, cwd, path ──┼──▶ entry changed? ──▶ revision + 1
//!  ledger row, side card (product) ┘                       `agent` frame
//! ```
//!
//! The epoch is new at each open of the tree, so a page that held the agents
//! before a restart of the backend reads them whole.
//!
//! A step of a session, one change of it, changes each entry once: what it
//! changes of one entry, such as its starting message and its phase, goes as
//! one `agent` frame at one revision when the step ends ([`Agents::step`]),
//! so no page sees an entry half changed
//! (`docs/decisions/0023-a-command-settles-on-the-state-it-changed.md`).

use std::{
    cell::{Cell, RefCell},
    collections::{HashMap, HashSet},
    rc::Rc,
};

use demi_agent_store::{NodeRecord, Placement, SideOf};
use demi_conversation_socket_protocol::{AgentCost, AgentEntry, AgentRole, JobModel, ServerFrame, SideQuestion, Version};
use demi_shared_types::{NodeId, SessionPhase};

use super::FrameSink;

/// A side question's card as the product keeps it: its title, null until
/// the small model wrote one, and whether it is folded.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SideCard {
    pub title: Option<String>,
    pub folded: bool,
}

/// What the product keeps of a conversation's agents that the tree shows: the
/// cost of each agent by its number, the root's 0, and each side question's
/// card.
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
    cards: RefCell<HashMap<NodeId, SideCard>>,
    /// The agents whose cost a ledger row announced since the tree opened,
    /// which what the product kept before that leaves as they are.
    announced: RefCell<HashSet<u64>>,
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
            cards: RefCell::default(),
            announced: RefCell::default(),
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
        self.sink.emit(ServerFrame::Agent { revision, agent: entry });
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

    /// The agent `number` costs `cost` now, as its latest ledger row made it.
    pub(crate) fn cost_changed(&self, number: u64, cost: AgentCost) {
        self.announced.borrow_mut().insert(number);
        self.costs.borrow_mut().insert(number, cost);
        let ids: Vec<NodeId> = self
            .entries
            .borrow()
            .values()
            .filter(|kept| number_of(&kept.entry.role) == Some(number))
            .map(|kept| kept.entry.id.clone())
            .collect();
        for id in ids {
            self.change(&id, |_| {});
        }
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

    /// The `agents` frame for a page that holds the agents at `held`: the
    /// entries changed since, with those that wait for their model, whose
    /// wait the page counts from the frame; every entry when it holds another
    /// epoch or none. `waits` tells how long each live agent has waited now.
    pub(crate) fn snapshot(&self, held: Option<&Version>, waits: impl Fn(&NodeId) -> Option<u64>) -> ServerFrame {
        let since = held
            .filter(|held| held.epoch == self.epoch && held.revision <= self.revision.get())
            .map(|held| held.revision);
        let mut agents: Vec<AgentEntry> = self
            .entries
            .borrow()
            .values()
            .filter_map(|kept| {
                let mut entry = kept.entry.clone();
                entry.waited_ms = entry.waited_ms.and_then(|_| waits(&entry.id));
                let changed = since.is_none_or(|since| kept.revision > since);
                (changed || entry.waited_ms.is_some()).then_some(entry)
            })
            .collect();
        agents.sort_by_key(rank);
        ServerFrame::Agents {
            version: self.version(),
            since,
            agents,
        }
    }

    /// Every entry, in the order the frames list them.
    pub(crate) fn all(&self) -> Vec<AgentEntry> {
        let mut agents: Vec<AgentEntry> = self.entries.borrow().values().map(|kept| kept.entry.clone()).collect();
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
