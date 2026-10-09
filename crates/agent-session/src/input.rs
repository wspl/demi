//! A session's waiting input (`runtime.md` § Input, § Yield wakeups): the
//! human steers it accepted, the agent messages it admitted and the yield
//! wakeups that fired, each waiting for a continuation boundary; and the
//! wakeups still scheduled. Pure state: the session decides when a boundary
//! takes them.

use demi_shared_types::{
    AgentMessage, BlockId, CommandEnd, CommandId, PendingSteer, Timestamp, WakeupCommand, WakeupId,
};
use tokio_util::task::AbortOnDropHandle;

use demi_agent_store::{PendingAgentInput, ScheduledWakeup};

/// One input waiting for a boundary.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Input {
    /// A human steer, which the pending steers list shows.
    Steer(PendingSteer),
    /// A yield wakeup that fired.
    Wakeup(ScheduledWakeup),
    /// A message from another agent of the tree, kept in the checkpoint.
    Agent(PendingAgentInput),
}

impl Input {
    fn id(&self) -> &str {
        match self {
            Self::Steer(steer) => steer.id.as_str(),
            Self::Wakeup(wakeup) => wakeup.id.as_str(),
            Self::Agent(input) => input.message.id.as_str(),
        }
    }
}

/// Which inputs a boundary writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Take {
    /// The user's Stop and a failure write all of it.
    Everything,
    /// A shutdown writes the human steers; the rest stays in the checkpoint.
    Steers,
}

impl Take {
    fn takes(self, input: &Input) -> bool {
        match self {
            Self::Everything => true,
            Self::Steers => matches!(input, Input::Steer(_)),
        }
    }
}

/// The inputs waiting, in the order they arrived.
#[derive(Debug, Default)]
pub(super) struct InputQueue {
    entries: Vec<Input>,
    /// How many inputs arrived, less those withdrawn: a turn that sees it
    /// grow during a round asks the provider once more, and a withdrawn steer
    /// causes no request.
    arrivals: u64,
    /// How many of them were agent messages and fired wakeups, which end a
    /// shell tool's window (`runtime.md` § The window).
    joining: u64,
}

impl InputQueue {
    /// A queue that starts with what a checkpoint kept.
    pub(super) fn restored(agent_inputs: Vec<PendingAgentInput>) -> Self {
        let mut queue = Self::default();
        for input in agent_inputs {
            queue.add(Input::Agent(input));
        }
        queue
    }

    pub(super) fn add(&mut self, input: Input) {
        if !matches!(input, Input::Steer(_)) {
            self.joining += 1;
        }
        self.entries.push(input);
        self.arrivals += 1;
    }

    pub(super) fn arrivals(&self) -> u64 {
        self.arrivals
    }

    pub(super) fn joining(&self) -> u64 {
        self.joining
    }

    /// Whether the human steer `id` is pending.
    pub(super) fn has_steer(&self, id: &BlockId) -> bool {
        self.entries
            .iter()
            .any(|input| matches!(input, Input::Steer(steer) if &steer.id == id))
    }

    /// Withdraws the pending human steer `id`; false when none is pending.
    pub(super) fn cancel_steer(&mut self, id: &BlockId) -> bool {
        let Some(index) = self
            .entries
            .iter()
            .position(|input| matches!(input, Input::Steer(steer) if &steer.id == id))
        else {
            return false;
        };
        self.entries.remove(index);
        self.arrivals = self.arrivals.saturating_sub(1);
        true
    }

    /// The human steers, as the pending steers list shows them.
    pub(super) fn pending_steers(&self) -> Vec<PendingSteer> {
        self.entries
            .iter()
            .filter_map(|input| match input {
                Input::Steer(steer) => Some(steer.clone()),
                _ => None,
            })
            .collect()
    }

    /// The agent messages waiting, as the checkpoint keeps them.
    pub(super) fn agent_inputs(&self) -> Vec<PendingAgentInput> {
        self.entries
            .iter()
            .filter_map(|input| match input {
                Input::Agent(input) => Some(input.clone()),
                _ => None,
            })
            .collect()
    }

    /// The wakeups that fired and wait to be written.
    pub(super) fn fired_wakeups(&self) -> impl Iterator<Item = &ScheduledWakeup> {
        self.entries.iter().filter_map(|input| match input {
            Input::Wakeup(wakeup) => Some(wakeup),
            _ => None,
        })
    }

    pub(super) fn has_agent_input(&self) -> bool {
        self.entries
            .iter()
            .any(|input| matches!(input, Input::Agent(_)))
    }

    pub(super) fn has_fired_wakeup(&self) -> bool {
        self.fired_wakeups().next().is_some()
    }

    /// Whether any input waits under `id`.
    pub(super) fn contains(&self, id: &str) -> bool {
        self.entries.iter().any(|input| input.id() == id)
    }

    /// The waiting agent message with this id.
    pub(super) fn agent_message(&self, id: &BlockId) -> Option<&AgentMessage> {
        self.entries.iter().find_map(|input| match input {
            Input::Agent(input) if &input.message.id == id => Some(&input.message),
            _ => None,
        })
    }

    /// Takes the inputs a boundary writes, in arrival order.
    pub(super) fn take(&mut self, take: Take) -> Vec<Input> {
        let (taken, kept) = std::mem::take(&mut self.entries)
            .into_iter()
            .partition(|input| take.takes(input));
        self.entries = kept;
        taken
    }

    /// Takes the first fired wakeup: the input a continuation opens with.
    pub(super) fn take_first_wakeup(&mut self) -> Option<ScheduledWakeup> {
        let index = self
            .entries
            .iter()
            .position(|input| matches!(input, Input::Wakeup(_)))?;
        match self.entries.remove(index) {
            Input::Wakeup(wakeup) => Some(wakeup),
            _ => unreachable!("the position is a wakeup's"),
        }
    }

    /// Drops the wakeups that fired and wait to be written.
    pub(super) fn drop_wakeups(&mut self) {
        self.entries
            .retain(|input| !matches!(input, Input::Wakeup(_)));
    }

    /// Drops the human steers still pending when their action ended
    /// normally.
    pub(super) fn discard_steers(&mut self) {
        self.entries
            .retain(|input| !matches!(input, Input::Steer(_)));
    }
}

/// The yield wakeups scheduled and not yet fired, in the order they were
/// scheduled, each with the task that watches the commands it names.
#[derive(Debug, Default)]
pub(super) struct Wakeups {
    scheduled: Vec<Scheduled>,
}

#[derive(Debug)]
struct Scheduled {
    wakeup: ScheduledWakeup,
    /// Watches the wakeup's commands until one ends; it goes with the
    /// wakeup, whether the wakeup fires or is cancelled.
    watch: Option<AbortOnDropHandle<()>>,
}

impl Wakeups {
    /// The wakeups a checkpoint saved, restored at `now`. Their commands
    /// ended with the shells of the session that saved them, so a wakeup
    /// that names commands and saw none of them end is due at once, as one
    /// that ended with its node's shells (`runtime.md` § Yield wakeups).
    pub(super) fn restored(scheduled: Vec<ScheduledWakeup>, now: Timestamp) -> Self {
        let mut wakeups = Self {
            scheduled: scheduled
                .into_iter()
                .map(|wakeup| Scheduled {
                    wakeup,
                    watch: None,
                })
                .collect(),
        };
        for scheduled in &mut wakeups.scheduled {
            let wakeup = &mut scheduled.wakeup;
            if wakeup.ended.is_none()
                && let Some(command_id) = wakeup.command_ids.first()
            {
                wakeup.ended = Some(WakeupCommand {
                    command_id: command_id.clone(),
                    end: CommandEnd::Stopped,
                });
                wakeup.due_at = wakeup.due_at.map(|due| due.min(now));
            }
        }
        wakeups
    }

    /// A wakeup whose wait starts when its action ends, unless one of
    /// `commands` ends first. It replaces the wakeup scheduled before it,
    /// which never fires: a session waits for one thing at a time, and an
    /// agent that yields again has decided anew what it waits for.
    pub(super) fn schedule(&mut self, id: WakeupId, duration_ms: u32, commands: Vec<CommandId>) {
        // Dropping the replaced wakeup's watch stops it.
        self.scheduled.clear();
        self.scheduled.push(Scheduled {
            wakeup: ScheduledWakeup {
                id,
                duration_ms,
                command_ids: commands,
                ended: None,
                due_at: None,
            },
            watch: None,
        });
    }

    /// Keeps the task that watches the commands of the wakeup `id`; a
    /// wakeup that is gone already drops it.
    pub(super) fn watch(&mut self, id: &WakeupId, watch: AbortOnDropHandle<()>) {
        if let Some(scheduled) = self.find(id) {
            scheduled.watch = Some(watch);
        }
    }

    /// One of the wakeup `id`'s commands ended, as `ended` says: the wakeup
    /// is due at once, or when its action ends, if it has not ended yet.
    /// A wakeup that fired or was cancelled already changes nothing.
    pub(super) fn command_ended(&mut self, id: &WakeupId, ended: WakeupCommand, now: Timestamp) {
        let Some(scheduled) = self.find(id) else {
            return;
        };
        // The watch that reports this ends by itself, and its handle goes
        // with the wakeup.
        let wakeup = &mut scheduled.wakeup;
        if wakeup.ended.is_none() {
            wakeup.ended = Some(ended);
        }
        wakeup.due_at = wakeup.due_at.map(|due| due.min(now));
    }

    fn find(&mut self, id: &WakeupId) -> Option<&mut Scheduled> {
        self.scheduled
            .iter_mut()
            .find(|scheduled| &scheduled.wakeup.id == id)
    }

    /// Starts the wait of every wakeup whose action ended: each is due its
    /// duration after `now`, or at `now` when one of its commands ended
    /// meanwhile.
    pub(super) fn arm(&mut self, now: Timestamp) {
        for Scheduled { wakeup, .. } in &mut self.scheduled {
            if wakeup.due_at.is_none() {
                wakeup.due_at = Some(match wakeup.ended {
                    Some(_) => now,
                    None => after(now, wakeup.duration_ms),
                });
            }
        }
    }

    /// When the next wakeup is due.
    pub(super) fn next_due(&self) -> Option<Timestamp> {
        self.scheduled
            .iter()
            .filter_map(|scheduled| scheduled.wakeup.due_at)
            .min()
    }

    /// Takes the wakeups due at `now`, in the order they were scheduled.
    pub(super) fn take_due(&mut self, now: Timestamp) -> Vec<ScheduledWakeup> {
        let (due, waiting): (Vec<Scheduled>, Vec<Scheduled>) = std::mem::take(&mut self.scheduled)
            .into_iter()
            .partition(|scheduled| scheduled.wakeup.due_at.is_some_and(|due| due <= now));
        self.scheduled = waiting;
        due.into_iter().map(|scheduled| scheduled.wakeup).collect()
    }

    /// Cancels the oldest scheduled wakeup; false when none is scheduled.
    pub(super) fn cancel_oldest(&mut self) -> bool {
        if self.scheduled.is_empty() {
            return false;
        }
        self.scheduled.remove(0);
        true
    }

    pub(super) fn is_empty(&self) -> bool {
        self.scheduled.is_empty()
    }

    pub(super) fn scheduled(&self) -> impl Iterator<Item = &ScheduledWakeup> {
        self.scheduled.iter().map(|scheduled| &scheduled.wakeup)
    }
}

/// The moment `duration_ms` after `now`.
fn after(now: Timestamp, duration_ms: u32) -> Timestamp {
    let due = now.as_millisecond() + i64::from(duration_ms);
    Timestamp::from_millisecond(due).expect("a wakeup is due within the supported range of times")
}
