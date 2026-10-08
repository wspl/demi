//! Input one agent of a tree sends another (`subagents.md` § Message
//! identity): an explicit message, or a child's completion receipt; and two
//! messages of the user's that reach an agent the same way: the decision on
//! a permission request, which reaches the agent that asked (`permissions.md`
//! § The decision's message), and a move of the conversation the user tells
//! the root of (`sessions-and-targets.md` § Switch the primary target).

use std::{fmt, str::FromStr};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{BlockId, MAX_SAFE_INTEGER, NodeId, Timestamp, is_blank};

/// A message between agents. The supervisor supplies the sender from the
/// invoking node, so a model cannot impersonate another sender.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentMessage {
    /// The id of the `agent_message` block the message becomes. A
    /// completion's id is its [`CompletionId`]; a permission decision's is
    /// `permission:<request id>`; a move's is `moved:<revision>`, the
    /// execution-context revision the move made.
    #[garde(custom(names_completed_round(&self.sender, &self.event)))]
    pub id: BlockId,
    /// The agent that sent it; none for the user, who sends only a
    /// permission decision and a move.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(custom(sent_by_an_agent_unless_the_users(&self.event)), dive)]
    pub sender: Option<Sender>,
    #[garde(skip)]
    pub recipient_id: NodeId,
    /// When the message was sent; for a completion, when the child closed.
    #[garde(skip)]
    pub timestamp: Timestamp,
    /// The body: a completion's result for `completed`, otherwise its failure
    /// text, which can be empty. An explicit message is never empty.
    #[garde(custom(has_body_when_explicit(&self.event)))]
    pub content: String,
    #[garde(dive)]
    pub event: AgentMessageEvent,
}

/// Who sent an agent message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Sender {
    #[garde(skip)]
    pub id: NodeId,
    /// The number the model knows the sender by: 0 for the root
    /// (`runtime.md` § Identifiers the model sees).
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub number: u64,
    /// The sender's description; `root session` for the root.
    #[garde(skip)]
    pub description: String,
    /// The sender's round: 1 for its first run, one more at each resume.
    #[garde(range(min = 1, max = MAX_SAFE_INTEGER))]
    pub round: u64,
}

/// What an agent message is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum AgentMessageEvent {
    /// An explicit communication between live agents.
    Message {},
    /// A supervisor's receipt of a child's end.
    Completion {
        #[garde(skip)]
        outcome: CompletionOutcome,
    },
    /// The user's decision on a permission request the recipient, or a
    /// subagent of it that closed since, raised.
    Permission {
        #[garde(skip)]
        outcome: PermissionOutcome,
        /// The category's action, such as `manage skills`, which the
        /// receipt row names.
        #[garde(length(min = 1))]
        action: String,
    },
    /// The user's move of the conversation, which the user told the root
    /// of; the receipt row names where it now runs.
    Moved {
        /// The Host it runs on now, by the name the user knows it by.
        #[garde(length(min = 1))]
        host: String,
        /// The directory its work starts in there.
        #[garde(length(min = 1))]
        path: String,
        /// The Host's home directory as its runner reported it, under which
        /// the receipt shortens the path to `~`; none before it reported one.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[garde(skip)]
        home: Option<String>,
    },
}

impl AgentMessageEvent {
    /// Whether the user sent the message: a permission decision or a move,
    /// which names no agent sender and wakes the recipient as the user's own
    /// input does.
    pub fn is_the_users(&self) -> bool {
        matches!(self, Self::Permission { .. } | Self::Moved { .. })
    }
}

/// How the user decided a permission request.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PermissionOutcome {
    Allowed,
    Denied,
}

serde_plain::derive_display_from_serialize!(PermissionOutcome);

/// How a child ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CompletionOutcome {
    Completed,
    Failed,
    Aborted,
}

serde_plain::derive_display_from_serialize!(CompletionOutcome);
serde_plain::derive_fromstr_from_deserialize!(CompletionOutcome);

/// The id of a child's completion receipt, `subagent:<child id>:<round>`: it
/// names exactly one execution round of one child, so reopening the child
/// starts a receipt of its own.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct CompletionId {
    pub child: NodeId,
    pub round: u64,
}

const COMPLETION_PREFIX: &str = "subagent:";

impl CompletionId {
    /// The id of the block the receipt becomes.
    pub fn block_id(&self) -> BlockId {
        BlockId::try_from(self.to_string()).expect("a completion id is never empty")
    }
}

impl fmt::Display for CompletionId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{COMPLETION_PREFIX}{}:{}",
            self.child, self.round
        )
    }
}

/// Why a text is not a [`CompletionId`].
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not a completion id (subagent:<child id>:<round>)")]
pub struct NotCompletionId(pub String);

impl FromStr for CompletionId {
    type Err = NotCompletionId;

    fn from_str(text: &str) -> Result<Self, NotCompletionId> {
        let refuse = || NotCompletionId(text.to_owned());
        let rest = text.strip_prefix(COMPLETION_PREFIX).ok_or_else(refuse)?;
        let (child, round) = rest.rsplit_once(':').ok_or_else(refuse)?;
        if round.is_empty() || !round.bytes().all(|byte| byte.is_ascii_digit()) {
            return Err(refuse());
        }
        let round: u64 = round.parse().map_err(|_| refuse())?;
        if round > MAX_SAFE_INTEGER {
            return Err(refuse());
        }
        let child = NodeId::try_from(child).map_err(|_| refuse())?;
        Ok(Self { child, round })
    }
}

/// The user sends a permission decision and a move, and an agent every
/// other message.
fn sent_by_an_agent_unless_the_users(
    event: &AgentMessageEvent,
) -> impl FnOnce(&Option<Sender>, &()) -> garde::Result + '_ {
    move |sender, _| {
        match (sender, event.is_the_users()) {
            (Some(_), true) => Err(garde::Error::new(
                "a permission decision or a move is the user's and names no agent sender",
            )),
            (None, false) => Err(garde::Error::new("an agent message names its sender")),
            _ => Ok(()),
        }
    }
}

/// A completion's id names the round of its sender that it completes.
fn names_completed_round<'a>(
    sender: &'a Option<Sender>,
    event: &'a AgentMessageEvent,
) -> impl FnOnce(&BlockId, &()) -> garde::Result + 'a {
    move |id, _| {
        let AgentMessageEvent::Completion { .. } = event else {
            return Ok(());
        };
        let Some(sender) = sender else {
            return Ok(());
        };
        let expected = CompletionId {
            child: sender.id.clone(),
            round: sender.round,
        };
        if id.as_str() != expected.to_string() {
            return Err(garde::Error::new(format!(
                "a completion's id must be {expected}, the round it completes"
            )));
        }
        Ok(())
    }
}

/// An explicit message, a permission decision and a move have a body.
fn has_body_when_explicit(
    event: &AgentMessageEvent,
) -> impl FnOnce(&str, &()) -> garde::Result + '_ {
    move |content, _| {
        if let AgentMessageEvent::Completion { .. } = event {
            return Ok(());
        }
        if is_blank(content) {
            return Err(garde::Error::new(
                "an explicit agent message must not be empty",
            ));
        }
        Ok(())
    }
}
