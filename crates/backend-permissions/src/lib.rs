//! Conversation permissions (`permissions.md`): the check every `rpc` call
//! passes in the backend's dispatch before its handler; the requests a
//! refused call raises; the user's decisions, the grants an allow records,
//! each kept for the conversation's life, and the message each decision
//! sends the agent that asked, delivered again at start when a restart cut
//! it off. And the conversation's other requests, which share the list, the
//! mark and the routes (`credentials.md` § Requests and routes): an agent's
//! request for a credential, which its command waits for, and a read of a
//! credential the conversation may not use yet, which asks for an allowance
//! as a refused command asks for a grant.
//!
//! Every command source passes through the check, a plugin's, the product's
//! and the agent runtime's alike, so it belongs to none of them. What it
//! needs of its user's shard it reaches through [`PermissionShard`].

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;

use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::credentials::{CredentialSeal, StoredCredential};
use demi_backend_database::permissions::{
    Asked, AskingAgent, Checked, CredentialAsk, CredentialRead, PermissionRefusal, StoredAsk,
    StoredRequest,
};
use demi_backend_page_sync::{Part, UserMarks};
use demi_command_declarations::Category;
use demi_host_interface::{RpcError, RpcInvocation};
use demi_shared_types::{AgentMessage, AgentMessageEvent, BlockId, NodeId, PermissionOutcome};
use demi_web_api_protocol::credentials::{Keep, RequestedValues};
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::permissions::{
    ConversationPermissions, PermissionCategory, PermissionDecision, PermissionRequest,
    PermissionRequestId,
};
use futures_util::future::LocalBoxFuture;
use tokio::sync::oneshot;

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

    /// The commands that wait for the answers to their credential requests.
    fn waits(&self) -> &Waits;

    /// What seals and opens the values of the user's credentials.
    fn credential_seal(&self) -> Arc<dyn CredentialSeal>;

    /// The user's credentials changed: every page receives them, and the
    /// expiry looks at their times again.
    fn credentials_changed(&self);
}

/// How a credential request was answered, which its waiting command
/// returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Answer {
    /// The user saved the credential, kept as `keep` says.
    Saved {
        credential: StoredCredential,
        keep: Keep,
    },
    /// The user declined.
    Declined,
    /// The request is gone without an answer: its conversation was
    /// archived.
    Withdrawn,
}

/// The commands that wait for the answers to their credential requests, by
/// request.
#[derive(Default)]
pub struct Waits(RefCell<HashMap<PermissionRequestId, Waiting>>);

struct Waiting {
    conversation: ConversationId,
    answer: oneshot::Sender<Answer>,
}

impl Waits {
    fn answer(&self, request: &PermissionRequestId, answer: Answer) {
        if let Some(waiting) = self.0.borrow_mut().remove(request) {
            // A command that stopped waiting withdraws its request itself.
            let _ = waiting.answer.send(answer);
        }
    }
}

/// A command's wait for the answer to its request: once it ends without
/// one, whether it was stopped or its handler dropped, the request is
/// withdrawn, since nobody waits for its answer.
struct Wait {
    shard: Rc<dyn PermissionShard>,
    conversation: ConversationId,
    request: PermissionRequestId,
}

impl Drop for Wait {
    fn drop(&mut self) {
        let waiting = self.shard.waits().0.borrow_mut().remove(&self.request);
        if waiting.is_none() {
            return;
        }
        let shard = self.shard.clone();
        let conversation = self.conversation.clone();
        let request = self.request.clone();
        self.shard.spawn(Box::pin(async move {
            match shard.control().withdraw_credential_request(request.clone()).await {
                Ok(true) => changed(&*shard, &conversation),
                Ok(false) => {}
                // The next start withdraws every credential request left.
                Err(error) => tracing::warn!(%request, "a credential request was not withdrawn: {error}"),
            }
        }));
    }
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
    #[error("{0}")]
    InvalidAnswer(String),
    #[error("{0}")]
    FieldsDiffer(String),
    #[error("The credential was removed")]
    Removed,
    #[error(transparent)]
    Storage(#[from] StorageError),
}

impl From<PermissionRefusal> for PermissionError {
    fn from(refusal: PermissionRefusal) -> Self {
        match refusal {
            PermissionRefusal::Archived => Self::Archived,
            PermissionRefusal::NotFound => Self::NotFound,
            PermissionRefusal::InvalidAnswer(reason) => Self::InvalidAnswer(reason),
            PermissionRefusal::FieldsDiffer(reason) => Self::FieldsDiffer(reason),
            PermissionRefusal::Removed => Self::Removed,
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
    let Asking {
        conversation,
        agent,
        command,
    } = asking(shard, invocation)?;
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

/// The conversation, the agent and the command line of a call that asks
/// the user.
struct Asking {
    conversation: ConversationId,
    agent: AskingAgent,
    command: String,
}

fn asking(shard: &dyn PermissionShard, invocation: &RpcInvocation) -> Result<Asking, RpcError> {
    let conversation = ConversationId::try_from(invocation.context.conversation.as_str())
        .map_err(|_| RpcError::Failed("the call names no conversation".into()))?;
    let node = invocation
        .caller
        .as_ref()
        .map(|caller| caller.node.clone())
        .ok_or_else(|| RpcError::Failed("a command that asks the user runs for an agent".into()))?;
    let agent = shard
        .asking_agent(&conversation, &node)
        .map_err(RpcError::Failed)?;
    Ok(Asking {
        conversation,
        agent,
        command: command_line(invocation)?,
    })
}

/// Asks the user for the credential `name` (`credentials.md` § Commands)
/// and waits for the answer, until `cancelled` resolves, which withdraws
/// the request. A credential of the name saved with other fields refuses
/// the request before the user sees it.
pub async fn ask_credential(
    shard: &dyn PermissionShard,
    invocation: &RpcInvocation,
    name: String,
    ask: CredentialAsk,
    cancelled: impl Future<Output = ()>,
) -> Result<Answer, RpcError> {
    let Asking {
        conversation,
        agent,
        command,
    } = asking(shard, invocation)?;
    let request = PermissionRequestId::try_from(uuid::Uuid::new_v4().to_string())
        .expect("a UUID is not empty");
    // Waiting before the request is stored, so an answer that comes at
    // once finds its command.
    let (answer, answered) = oneshot::channel();
    shard.waits().0.borrow_mut().insert(
        request.clone(),
        Waiting {
            conversation: conversation.clone(),
            answer,
        },
    );
    let _wait = Wait {
        shard: shard.this(),
        conversation: conversation.clone(),
        request: request.clone(),
    };
    let asked = shard
        .control()
        .ask_credential(conversation.clone(), request, name.clone(), ask, command, agent)
        .await
        .map_err(|error| RpcError::Failed(error.to_string()))?;
    if let Asked::FieldsDiffer(fields) = asked {
        let fields = fields
            .iter()
            .map(|field| format!("{}:{}", field.name, field.kind.as_str()))
            .collect::<Vec<_>>()
            .join(", ");
        return Err(RpcError::Failed(format!(
            "\u{201c}{name}\u{201d} is saved with other fields ({fields}): ask for those fields, or under another name"
        )));
    }
    changed(shard, &conversation);
    tokio::select! {
        // The sender goes only with the answer or a withdrawal.
        answer = answered => Ok(answer.unwrap_or(Answer::Withdrawn)),
        () = cancelled => Err(RpcError::Failed("the call was cancelled".into())),
    }
}

/// Answers the waiting commands of the conversation's credential requests,
/// which an archive withdrew.
pub fn withdrawn(shard: &dyn PermissionShard, conversation: &ConversationId) {
    let requests: Vec<PermissionRequestId> = shard
        .waits()
        .0
        .borrow()
        .iter()
        .filter(|(_, waiting)| waiting.conversation == *conversation)
        .map(|(request, _)| request.clone())
        .collect();
    for request in requests {
        shard.waits().answer(&request, Answer::Withdrawn);
    }
}

/// The field `field` of the credential `name`, for the call's agent
/// (`credentials.md` § Using a credential): its value when the conversation
/// may use the credential. Otherwise it records the allowance request and
/// fails the call as a refused permission does.
pub async fn read_credential(
    shard: &dyn PermissionShard,
    invocation: &RpcInvocation,
    name: String,
    field: String,
) -> Result<String, RpcError> {
    let Asking {
        conversation,
        agent,
        command,
    } = asking(shard, invocation)?;
    let read = shard
        .control()
        .read_credential(
            conversation.clone(),
            name.clone(),
            field.clone(),
            command,
            agent,
            shard.credential_seal(),
        )
        .await
        .map_err(|error| RpcError::Failed(error.to_string()))?;
    match read {
        CredentialRead::Value(value) => {
            shard.credentials_changed();
            Ok(value)
        }
        CredentialRead::NoCredential => Err(RpcError::Failed(format!(
            "\u{201c}{name}\u{201d}: the user has no credential of that name; `demi credentials list` lists them"
        ))),
        CredentialRead::NoField(fields) => Err(RpcError::Failed(format!(
            "\u{201c}{name}\u{201d}: no field \u{201c}{field}\u{201d}; its fields are {}",
            fields
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
        CredentialRead::Raised(saved) => {
            changed(shard, &conversation);
            Err(RpcError::Failed(format!(
                "this conversation needs the user's permission to {}; the request was sent to the user, and you will be told when the user decides",
                use_action(&saved)
            )))
        }
    }
}

/// What an allowance lets the conversation do, as the card's title, the
/// refusal and the decision's message say it.
fn use_action(name: &str) -> String {
    format!("use the credential \u{201c}{name}\u{201d}")
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
        .map_err(|error| RpcError::Failed(error.to_string()))
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
            .map(|request| {
                let (id, command, agent, created_at) = (
                    request.id,
                    request.command,
                    request.agent.subagent,
                    request.created_at,
                );
                match request.ask {
                    StoredAsk::Permission { categories } => PermissionRequest::Permission {
                        id,
                        categories: categories.iter().map(|id| category(id)).collect(),
                        command,
                        agent,
                        created_at,
                    },
                    StoredAsk::Credential { name, ask } => PermissionRequest::Credential {
                        id,
                        name,
                        reason: ask.reason,
                        fields: ask.fields,
                        keep: ask.keep,
                        command,
                        agent,
                        created_at,
                    },
                    StoredAsk::Allowance { name, credential } => PermissionRequest::Allowance {
                        id,
                        name,
                        removed: credential.is_none(),
                        command,
                        agent,
                        created_at,
                    },
                }
            })
            .collect(),
    })
}

/// Decides the undecided request `request` (`permissions.md` § Requests):
/// an allow of a permission request grants its categories to the
/// conversation and decides each request whose categories are then all
/// granted, an allow of an allowance request allows its credential and
/// decides each request to use it, a deny decides this one alone. Each
/// agent that asked is sent its message after the answer. A credential
/// request is allowed with `credential`, its values and keep, which saves
/// the credential and allows it in the conversation; its command, which
/// waits, returns with the answer instead of a message.
pub async fn decide(
    shard: &dyn PermissionShard,
    conversation: &ConversationId,
    request: PermissionRequestId,
    decision: PermissionDecision,
    credential: Option<RequestedValues>,
) -> Result<(), PermissionError> {
    match (decision, credential) {
        (PermissionDecision::Allow, Some(RequestedValues { values, keep })) => {
            let credential = shard
                .control()
                .answer_credential(
                    conversation.clone(),
                    request.clone(),
                    values,
                    keep,
                    shard.credential_seal(),
                )
                .await??;
            changed(shard, conversation);
            shard.credentials_changed();
            shard.waits().answer(
                &request,
                Answer::Saved { credential, keep },
            );
            Ok(())
        }
        (PermissionDecision::Deny, Some(_)) => Err(PermissionError::InvalidAnswer(
            "credential: a deny carries no credential".into(),
        )),
        (decision, None) => {
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
                if let StoredAsk::Credential { .. } = request.ask {
                    shard.waits().answer(&request.id, Answer::Declined);
                    continue;
                }
                let this = shard.this();
                shard.spawn(Box::pin(async move { deliver(this, request).await }));
            }
            Ok(())
        }
    }
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
    let action = match &request.ask {
        StoredAsk::Permission { categories: ids } => {
            let categories = declared(&*shard).await;
            joined(ids.iter().map(|id| {
                categories
                    .iter()
                    .find(|category| category.id == *id)
                    .map_or(id.as_str(), |category| category.action.as_str())
            }))
        }
        StoredAsk::Allowance { name, .. } => use_action(name),
        // A credential request's command returns its answer.
        StoredAsk::Credential { .. } => return,
    };
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
        origin: None,
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
