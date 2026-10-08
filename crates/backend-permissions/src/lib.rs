//! Conversation permissions (`permissions.md`): the check every `rpc` call
//! passes in the backend's dispatch before its handler; the requests a
//! refused call raises; the user's decisions, the grants an allow records,
//! each kept for the conversation's life, and the message each decision
//! sends the agent that asked, delivered again at start when a restart cut
//! it off.
//!
//! Every command source passes through the check, a plugin's, the product's
//! and the agent runtime's alike, so it belongs to none of them. What it
//! needs of its user's shard it reaches through [`PermissionShard`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;

use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::permissions::{
    AskingAgent, Checked, PermissionRefusal, StoredRequest,
};
use demi_backend_page_sync::{Part, UserMarks};
use demi_command_declarations::Category;
use demi_host_interface::{RpcError, RpcInvocation};
use demi_shared_types::{AgentMessage, AgentMessageEvent, BlockId, NodeId, PermissionOutcome};
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::permissions::{
    ConversationPermissions, PermissionCategory, PermissionDecision, PermissionRequest,
    PermissionRequestId,
};
use futures_util::future::LocalBoxFuture;

/// What the permissions need of their user's shard.
pub trait PermissionShard {
    fn control(&self) -> &ControlService;

    /// The marks of the user's changes, which reach each of the user's
    /// pages.
    fn marks(&self) -> UserMarks;

    /// The revision of each conversation's requests.
    fn revisions(&self) -> &Revisions;

    /// The permission categories of the command set the user's
    /// conversations open with.
    fn categories(&self) -> LocalBoxFuture<'_, Result<Vec<Category>, String>>;

    /// The agent `node` of the conversation's open tree, as a request names
    /// it.
    fn asking_agent(
        &self,
        conversation: &ConversationId,
        node: &NodeId,
    ) -> Result<AskingAgent, String>;

    /// Admits the message `message` builds for its recipient into the agent
    /// `asking` of the conversation, or into its nearest live ancestor when
    /// that agent is closed, opening the conversation's tree as an open does
    /// when it is closed. Answers the recipient once the message is in its
    /// checkpoint.
    fn admit<'a>(
        &'a self,
        conversation: &'a ConversationId,
        asking: &'a NodeId,
        message: &'a dyn Fn(&NodeId) -> AgentMessage,
    ) -> LocalBoxFuture<'a, Result<NodeId, String>>;

    /// Runs `task` as a task of the shard, which the shard's close waits
    /// for.
    fn spawn(&self, task: LocalBoxFuture<'static, ()>);

    /// The shard, for a task that outlives the call that starts it.
    fn this(&self) -> Rc<dyn PermissionShard>;
}

/// How many times each conversation's requests changed since the shard
/// started; a conversation without an entry has changed none
/// (`web-api.md` § Conversation permissions).
#[derive(Default)]
pub struct Revisions(RefCell<HashMap<ConversationId, u64>>);

impl Revisions {
    pub fn of(&self, conversation: &ConversationId) -> u64 {
        self.0.borrow().get(conversation).copied().unwrap_or(0)
    }

    fn raise(&self, conversation: &ConversationId) {
        *self.0.borrow_mut().entry(conversation.clone()).or_default() += 1;
    }
}

/// Why a page's read or decision failed.
#[derive(Debug, thiserror::Error)]
pub enum PermissionError {
    #[error("The conversation is archived")]
    Archived,
    #[error("The conversation has no undecided permission request of that id")]
    NotFound,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl From<PermissionRefusal> for PermissionError {
    fn from(refusal: PermissionRefusal) -> Self {
        match refusal {
            PermissionRefusal::Archived => Self::Archived,
            PermissionRefusal::NotFound => Self::NotFound,
        }
    }
}

/// The check of one `rpc` call that needs `categories`, one or more
/// (`permissions.md` § The check): it passes when the call's conversation
/// has the grant of each, whichever agent of its tree runs it. Otherwise it
/// records one request of the categories the conversation lacks and fails
/// the call with the message the agent reads, and no handler runs.
pub async fn check(
    shard: &dyn PermissionShard,
    invocation: &RpcInvocation,
    categories: &[&Category],
) -> Result<(), RpcError> {
    let conversation = ConversationId::try_from(invocation.context.conversation.as_str())
        .map_err(|_| RpcError::Failed("the call names no conversation".into()))?;
    let node = invocation
        .caller
        .as_ref()
        .map(|caller| caller.node.clone())
        .ok_or_else(|| RpcError::Failed("a command that needs a permission runs for an agent".into()))?;
    let agent = shard
        .asking_agent(&conversation, &node)
        .map_err(RpcError::Failed)?;
    let command = command_line(invocation)?;
    let ids = categories.iter().map(|category| category.id.clone()).collect();
    let checked = shard
        .control()
        .check_permission(conversation.clone(), ids, command, agent)
        .await
        .map_err(|error| RpcError::Failed(error.to_string()))?;
    match checked {
        Checked::Granted => Ok(()),
        Checked::Raised { lacking, .. } => {
            changed(shard, &conversation);
            let lacking: Vec<&Category> = lacking
                .iter()
                .filter_map(|id| categories.iter().find(|category| category.id == *id).copied())
                .collect();
            Err(RpcError::Failed(format!(
                "this conversation needs the user's permission to {}; the request was sent to the user, and you will be told when the user decides",
                joined(lacking.iter().map(|category| category.action.as_str()))
            )))
        }
    }
}

/// The actions of several categories as one phrase: joined with "and", as
/// the card's title and the decision's message join them (`permissions.md`
/// § Several categories).
fn joined<'a>(actions: impl Iterator<Item = &'a str>) -> String {
    actions.collect::<Vec<_>>().join(" and ")
}

/// The command line as the agent ran it: the call's argv, which starts with
/// the root's name, quoted for a POSIX shell.
fn command_line(invocation: &RpcInvocation) -> Result<String, RpcError> {
    shlex::try_join(invocation.argv.iter().map(String::as_str))
        .map_err(|error| RpcError::Usage(error.to_string()))
}

/// The conversation's undecided requests, as a page reads them, with the
/// revision read before them: a change made during the read raises the
/// revision past it.
pub async fn read(
    shard: &dyn PermissionShard,
    conversation: &ConversationId,
) -> Result<ConversationPermissions, PermissionError> {
    let revision = shard.revisions().of(conversation);
    let requests = shard.control().permission_requests(conversation.clone()).await?;
    let categories = declared(shard).await;
    let category = |id: &str| {
        let declared = categories.iter().find(|category| category.id == id);
        PermissionCategory {
            id: id.to_owned(),
            action: declared.map(|category| category.action.clone()),
            description: declared.map(|category| category.description.clone()),
        }
    };
    Ok(ConversationPermissions {
        revision,
        requests: requests
            .into_iter()
            .map(|request| PermissionRequest {
                categories: request.categories.iter().map(|id| category(id)).collect(),
                id: request.id,
                command: request.command,
                agent: request.agent.subagent,
                created_at: request.created_at,
            })
            .collect(),
    })
}

/// Decides the undecided request `request` (`permissions.md` § Requests):
/// an allow grants its categories to the conversation and decides each
/// request whose categories are then all granted, a deny decides this one
/// alone. Each agent that asked is
/// sent its message after the answer.
pub async fn decide(
    shard: &dyn PermissionShard,
    conversation: &ConversationId,
    request: PermissionRequestId,
    decision: PermissionDecision,
) -> Result<(), PermissionError> {
    let outcome = match decision {
        PermissionDecision::Allow => PermissionOutcome::Allowed,
        PermissionDecision::Deny => PermissionOutcome::Denied,
    };
    let decided = shard
        .control()
        .decide_permission(conversation.clone(), request, outcome)
        .await??;
    changed(shard, conversation);
    for request in decided {
        let this = shard.this();
        shard.spawn(Box::pin(async move { deliver(this, request).await }));
    }
    Ok(())
}

/// The conversation's requests changed, as an archive that
/// withdrew its requests changes them: its revision rises and every page
/// receives its summary.
pub fn changed(shard: &dyn PermissionShard, conversation: &ConversationId) {
    shard.revisions().raise(conversation);
    shard.marks().mark(Part::Conversation(conversation.clone()));
}

/// Sends a decided request's message to the agent that asked, and forgets
/// the request once the message is in the agent's checkpoint. A delivery
/// that fails leaves the request stored, and the next start delivers it.
pub async fn deliver(shard: Rc<dyn PermissionShard>, request: StoredRequest) {
    let Some(decision) = request.decision else {
        return;
    };
    let categories = declared(&*shard).await;
    let action = joined(request.categories.iter().map(|id| {
        categories
            .iter()
            .find(|category| category.id == *id)
            .map_or(id.as_str(), |category| category.action.as_str())
    }));
    let message = |recipient: &NodeId| AgentMessage {
        id: BlockId::try_from(format!("permission:{}", request.id))
            .expect("a permission message's id is not empty"),
        sender: None,
        recipient_id: recipient.clone(),
        timestamp: decision.at,
        content: content(&request, decision.outcome, &action, recipient),
        event: AgentMessageEvent::Permission {
            outcome: decision.outcome,
            action: action.clone(),
        },
    };
    let admitted = shard
        .admit(&request.conversation, &request.agent.node, &message)
        .await;
    match admitted {
        Ok(_) => {
            if let Err(error) = shard.control().delivered_permission(request.id.clone()).await {
                // The request stays decided, and the next start delivers it
                // again under its id, which the agent admits once.
                tracing::warn!(request = %request.id, "a delivered permission decision was not forgotten: {error}");
            }
        }
        Err(error) => {
            tracing::warn!(
                request = %request.id,
                conversation = %request.conversation,
                "a permission decision was not delivered; the next start delivers it: {error}"
            );
        }
    }
}

/// The message the agent reads (`permissions.md` § The decision's message);
/// one delivered to an ancestor of the closed subagent that asked names it.
fn content(
    request: &StoredRequest,
    outcome: PermissionOutcome,
    action: &str,
    recipient: &NodeId,
) -> String {
    let command = &request.command;
    let mut text = match outcome {
        PermissionOutcome::Allowed => format!(
            "The user allowed this conversation to {action}; the command `{command}` can now run."
        ),
        PermissionOutcome::Denied => format!(
            "The user denied this conversation permission to {action}; the command `{command}` was not run."
        ),
    };
    if recipient != &request.agent.node
        && let Some(subagent) = &request.agent.subagent
    {
        text.push_str(&format!(
            " Agent {} ({}) ran it and has closed since.",
            subagent.number, subagent.description
        ));
    }
    text
}

/// The categories the user's command set declares; none when the set
/// cannot be composed now, which shows each category by its id.
async fn declared(shard: &dyn PermissionShard) -> Vec<Category> {
    match shard.categories().await {
        Ok(categories) => categories,
        Err(error) => {
            tracing::warn!("the permission categories were not read: {error}");
            Vec::new()
        }
    }
}
