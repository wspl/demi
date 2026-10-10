//! A session's waiting input (`runtime.md` § Input, § Command reports): the
//! human steers it accepted, the agent messages it admitted and the command
//! reports that arrived, each waiting for a continuation boundary. Pure
//! state: the session decides when a boundary takes them.

use demi_shared_types::{AgentMessage, BlockId, CommandReport, PendingSteer};

use demi_agent_store::PendingAgentInput;

/// One input waiting for a boundary.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Input {
    /// A human steer, which the pending steers list shows.
    Steer(PendingSteer),
    /// A command report, kept in the checkpoint.
    Report(CommandReport),
    /// A message from another agent of the tree, kept in the checkpoint.
    Agent(PendingAgentInput),
}

impl Input {
    /// The id it waits under; a report has none.
    fn id(&self) -> Option<&str> {
        match self {
            Self::Steer(steer) => Some(steer.id.as_str()),
            Self::Report(_) => None,
            Self::Agent(input) => Some(input.message.id.as_str()),
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
    /// How many of them were agent messages and command reports, which end
    /// a shell call's window (`runtime.md` § The window).
    joining: u64,
}

impl InputQueue {
    /// A queue that starts with what a checkpoint kept.
    pub(super) fn restored(agent_inputs: Vec<PendingAgentInput>, reports: Vec<CommandReport>) -> Self {
        let mut queue = Self::default();
        for input in agent_inputs {
            queue.add(Input::Agent(input));
        }
        for report in reports {
            queue.add(Input::Report(report));
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

    /// The command reports waiting, as the checkpoint keeps them.
    pub(super) fn reports(&self) -> Vec<CommandReport> {
        self.entries
            .iter()
            .filter_map(|input| match input {
                Input::Report(report) => Some(report.clone()),
                _ => None,
            })
            .collect()
    }

    pub(super) fn has_agent_input(&self) -> bool {
        self.entries
            .iter()
            .any(|input| matches!(input, Input::Agent(_)))
    }

    pub(super) fn has_report(&self) -> bool {
        self.entries
            .iter()
            .any(|input| matches!(input, Input::Report(_)))
    }

    /// Whether any input waits under `id`.
    pub(super) fn contains(&self, id: &str) -> bool {
        self.entries.iter().any(|input| input.id() == Some(id))
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

    /// Takes the command reports: the input a continuation opens with.
    pub(super) fn take_reports(&mut self) -> Vec<CommandReport> {
        self.entries
            .extract_if(.., |input| matches!(input, Input::Report(_)))
            .filter_map(|input| match input {
                Input::Report(report) => Some(report),
                _ => None,
            })
            .collect()
    }

    /// Drops the human steers still pending when their action ended
    /// normally.
    pub(super) fn discard_steers(&mut self) {
        self.entries
            .retain(|input| !matches!(input, Input::Steer(_)));
    }
}
