//! Input one agent of a tree sends another (`subagents.md` § Message
//! identity): an explicit message, or a child's completion receipt.

use std::{fmt, str::FromStr};

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{BlockId, MAX_SAFE_INTEGER, NodeId, Timestamp, is_blank};

/// A message between agents. The supervisor supplies the sender from the
/// invoking node, so a model cannot impersonate another sender. The field
/// order is the order replay writes the message as JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentMessage {
    /// The id of the `agent_message` block the message becomes. A
    /// completion's id is its [`CompletionId`].
    #[garde(custom(names_completed_round(&self.sender, &self.event)))]
    pub id: BlockId,
    #[garde(dive)]
    pub sender: Sender,
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
    /// The sender's description; `root session` for the root.
    #[garde(skip)]
    pub description: String,
    /// The sender node's persisted spawn time in milliseconds, which names
    /// one execution round of it.
    #[garde(range(max = MAX_SAFE_INTEGER))]
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
}

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
        write!(formatter, "{COMPLETION_PREFIX}{}:{}", self.child, self.round)
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

/// A completion's id names the round of its sender that it completes.
fn names_completed_round<'a>(
    sender: &'a Sender,
    event: &'a AgentMessageEvent,
) -> impl FnOnce(&BlockId, &()) -> garde::Result + 'a {
    move |id, _| {
        let AgentMessageEvent::Completion { .. } = event else {
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

/// An explicit message has a body.
fn has_body_when_explicit(
    event: &AgentMessageEvent,
) -> impl FnOnce(&str, &()) -> garde::Result + '_ {
    move |content, _| {
        let AgentMessageEvent::Message {} = event else {
            return Ok(());
        };
        if is_blank(content) {
            return Err(garde::Error::new("an explicit agent message must not be empty"));
        }
        Ok(())
    }
}
