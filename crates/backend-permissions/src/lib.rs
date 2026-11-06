//! Conversation permissions: the check every `rpc` call passes in the backend's
//! dispatch before its handler.
//!
//! The crate holds:
//!
//! - The check itself.
//! - The requests a call without a grant raises, which the call waits on until
//!   the user decides.
//! - The user's decisions, which end those waits.
//! - The grants an allow records, each kept for the conversation's life.
//! - The message each decision sends the agent that asked, delivered again at
//!   start when a restart cut it off.
//!
//! The crate also serves an agent's request for a credential, which its
//! command waits for and which shares the list, the mark and the routes.
//!
//! Every command source passes through the check, a plugin's, the product's and
//! the agent runtime's alike, so the check belongs to none of them. What it
//! needs of its user's shard it reaches through [`PermissionShard`].
//!
//! Demi has no general permission system. An agent runs commands on its Host,
//! edits files and opens pages without asking, because the user chose the Host
//! and the directory. It asks only before acting on Demi itself, on what the
//! user owns beyond the conversation. Each such power is one broad category,
//! which the user judges once for a conversation and which covers every object
//! and every step of it: every credential, every device.
//!
//! ```text
//! demi skills add ... -> dispatch: the check -- grant? -- yes --> handler
//!                                      | no
//!                          request recorded, call held (a running command)
//!                                      | the user: Allow / Deny
//!             grant stored, message to the agent that asked, call goes on
//! ```
//!
//! A request records one refused call:
//!
//! - A newer request of the same categories from the same agent replaces an
//!   undecided one, since the agent ran the command again and its newest line
//!   is the one to show.
//! - Allow records the grant of each of the request's categories. It decides as
//!   allowed every request of the conversation whose categories are now
//!   granted.
//! - Deny decides that one request and records nothing, so the next attempt
//!   raises a new request.
//! - Requests never expire. They survive a restart, and archiving withdraws
//!   them.
//!
//! A grant covers every agent of the conversation's tree, those spawned later
//! included. It lasts the conversation's life and survives an archive. A Fork
//! copies no grant, since the user decides for a new conversation again.
//!
//! The call holds while the user decides: it does not return and its window
//! does not run, so the agent gets nothing back until the user has answered. The
//! window starts from the allow.
//!
//! - Anything but Allow denies the request of a waiting call ([`deny`]): Deny,
//!   and the user's actions on the agent that asked, a message to it, queued,
//!   sent now or as a steer, its Edit and Send, Regenerate and Retry, its
//!   Stop, and the stop of the waiting command. The shard names the requests
//!   such an action stops ([`held_requests`]) and denies them once the agent
//!   has taken it, but for a Stop's, denied before it so that the agent reads
//!   them in the turn the stop ends.
//! - The command sends no silence report, since nothing about it changes until
//!   the user answers and the report would wake the agent for nothing. Its
//!   silence counts again from the end of the wait.
//! - A call that ends before the decision, stopped by its parent or lost with a
//!   restart, leaves its request as it is. An Allow still grants the category
//!   for the commands that come later.
//!
//! The crate decides whether a call is dispatched, and the shard dispatches it.
//! The crate runs no handler, knows no plugin, reaches no Host and sees no
//! agent session and no `Shard`.

use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;

use demi_core_store::StorageError;
use demi_core_store::store::CoreStore;
use demi_core_store::credentials::{CredentialSeal, StoredCredential};
use demi_core_store::permissions::{
    AnswerProblem, Answered, Asked, AskingAgent, Checked, CredentialAsk, CredentialRead,
    Needed, PermissionRefusal, SavedByAgent, StoredAsk, StoredRequest, Undecided,
};
use demi_backend_page_sync::{Part, Revisions, UserMarks};
use demi_command_declarations::Category;
use demi_host_interface::{PortError, RpcError, RpcInvocation, RpcPort};
use demi_shared_types::{AgentMessage, AgentMessageEvent, BlockId, NodeId, PermissionOutcome};
use demi_web_api_protocol::credentials::{CredentialField, Keep, RequestedValues};
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use demi_web_api_protocol::permissions::{
    ConversationPermissions, PermissionCategory, PermissionDecision, PermissionRequest,
    PermissionRequestId,
};
use demi_web_api_protocol::push::PushAsk;
use futures_util::future::LocalBoxFuture;
use tokio::sync::oneshot;

/// What the permissions need of their user's shard.
pub trait PermissionShard {
    fn store(&self) -> &CoreStore;

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
    /// that agent is closed.
    ///
    /// Opens the conversation's tree, as an open does, when the tree is closed.
    /// Answers the recipient once the message is in its checkpoint.
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

    /// The calls that wait for the answers to their requests.
    fn waits(&self) -> &Waits;

    /// What seals and opens the values of the user's credentials.
    fn credential_seal(&self) -> Arc<dyn CredentialSeal>;

    /// The user's credentials changed: every page receives them, and the
    /// expiry looks at their times again.
    fn credentials_changed(&self);

    /// The conversation asks the user what `ask` says, by the request
    /// `request`, which the user's browsers are notified of.
    fn asked(&self, conversation: &ConversationId, request: PermissionRequestId, ask: PushAsk);

    /// A call's request is stored and its pages told, before the call waits for
    /// the answer.
    ///
    /// A test holds the check here, to decide the request meanwhile. Nothing
    /// happens here otherwise.
    fn raised(&self) -> LocalBoxFuture<'_, ()>;
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

/// How the wait of a call for its permission request ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Decided {
    Allowed,
    Denied,
    /// A newer request of the same agent and need took its place.
    Replaced,
}

/// The calls that wait for the answers to their requests, by request: a
/// credential request's command, and a call held for a permission request.
#[derive(Default)]
pub struct Waits(RefCell<HashMap<PermissionRequestId, Waiting>>);

struct Waiting {
    conversation: ConversationId,
    answer: Answering,
}

/// What a waiting call is told.
enum Answering {
    Credential(oneshot::Sender<Answer>),
    Permission {
        /// The agent that ran the call and the provider's id of its `shell`
        /// call, which tell what a user's action on either stops.
        holder: Holder,
        answer: oneshot::Sender<Decided>,
    },
}

/// Whose call a permission request holds.
struct Holder {
    agent: NodeId,
    call: String,
}

/// Which held calls an action of the user stops.
pub enum Stops<'a> {
    /// The calls of these agents: a message to one of them, or its Stop.
    Agents(&'a [NodeId]),
    /// The one `shell` call, by the provider's id, whose command the user
    /// stopped.
    Call(&'a str),
}

impl Waits {
    fn wait(&self, request: PermissionRequestId, conversation: ConversationId, answer: Answering) {
        self.0.borrow_mut().insert(request, Waiting { conversation, answer });
    }

    /// Answers the command of the credential request `request`.
    fn answer(&self, request: &PermissionRequestId, answer: Answer) {
        if let Some(Answering::Credential(sender)) = self.take(request) {
            // A command that stopped waiting withdraws its request itself.
            let _ = sender.send(answer);
        }
    }

    /// The permission requests of `conversation` that hold a call `stops`
    /// names.
    fn held_by(&self, conversation: &ConversationId, stops: &Stops<'_>) -> Vec<PermissionRequestId> {
        self.0
            .borrow()
            .iter()
            .filter(|(_, waiting)| &waiting.conversation == conversation)
            .filter(|(_, waiting)| match (&waiting.answer, stops) {
                (Answering::Permission { holder, .. }, Stops::Agents(agents)) => agents.contains(&holder.agent),
                (Answering::Permission { holder, .. }, Stops::Call(call)) => holder.call == *call,
                (Answering::Credential(_), _) => false,
            })
            .map(|(request, _)| request.clone())
            .collect()
    }

    /// Ends the wait of the call held for the permission request `request`.
    fn decided(&self, request: &PermissionRequestId, decided: Decided) {
        if let Some(Answering::Permission { answer, .. }) = self.take(request) {
            // A call that stopped waiting leaves its request as it is.
            let _ = answer.send(decided);
        }
    }

    fn take(&self, request: &PermissionRequestId) -> Option<Answering> {
        self.0.borrow_mut().remove(request).map(|waiting| waiting.answer)
    }
}

/// A call's wait for the answer to its request, which ends its entry in
/// [`Waits`] however the call ends.
///
/// When the call ends without an answer:
///
/// - A credential request is withdrawn, whether its command was stopped or its
///   handler dropped, since nobody waits for it.
/// - A permission request stays for the user to decide.
struct Wait {
    shard: Rc<dyn PermissionShard>,
    conversation: ConversationId,
    request: PermissionRequestId,
}

impl Drop for Wait {
    fn drop(&mut self) {
        let Some(Answering::Credential(_)) = self.shard.waits().take(&self.request) else {
            return;
        };
        let shard = self.shard.clone();
        let conversation = self.conversation.clone();
        let request = self.request.clone();
        self.shard.spawn(Box::pin(async move {
            match shard.store().withdraw_credential_request(request.clone()).await {
                Ok(true) => changed(&*shard, &conversation),
                Ok(false) => {}
                // The next start withdraws every credential request left.
                Err(error) => tracing::warn!(event = "credential_request.withdraw_failed", %request, %error, "a credential request was not withdrawn"),
            }
        }));
    }
}

/// Why a page's read or decision failed. A credential's refusals are in
/// words for the user, who reads them as they are.
#[derive(Debug, thiserror::Error)]
pub enum PermissionError {
    #[error("The conversation is archived")]
    Archived,
    #[error("The conversation has no undecided permission request of that id")]
    NotFound,
    #[error("{0}")]
    InvalidAnswer(AnswerProblem),
    #[error("{0}")]
    FieldsDiffer(String),
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
        }
    }
}

/// What one `rpc` call needs of its conversation: its leaf's category, or
/// the device it brings in, which the category of the user's devices
/// covers; no command needs both.
pub enum Need<'a> {
    Category(&'a Category),
    Device(DeviceId),
}

/// The check of one `rpc` call that needs `need`.
///
/// The call passes when the conversation has the grant of the category, or
/// reaches the device, whichever agent of its tree runs it. Otherwise the check
/// records the request and holds the call until the user decides, the call's
/// job telling that it waits. No handler runs meanwhile.
///
/// - An allow passes the call.
/// - A deny, or a newer request of the same agent and need, fails the call with
///   the line the agent reads.
///
/// A call stopped meanwhile fails, an allow ready at the same moment
/// notwithstanding, and leaves its request to the user.
pub async fn check(
    shard: &dyn PermissionShard,
    invocation: &RpcInvocation,
    need: Need<'_>,
    port: &RpcPort,
) -> Result<(), RpcError> {
    let Asking {
        conversation,
        agent,
        call,
        command,
    } = asking(shard, invocation)?;
    let needed = match need {
        Need::Category(category) => Needed::Category(category.id.clone()),
        Need::Device(device) => Needed::Device(device),
    };
    let action = category_action(shard, needed.category()).await;
    let ask = PushAsk::Permission {
        categories: vec![PermissionCategory {
            id: needed.category().to_owned(),
            action: Some(action.clone()),
            description: None,
        }],
    };
    let request = new_request_id();
    // Waiting before the request is stored, so that an allow of another
    // request that grants its need finds the call once the request is
    // there, whatever the order of the two transactions.
    let (answer, decided) = oneshot::channel();
    shard
        .waits()
        .wait(
            request.clone(),
            conversation.clone(),
            Answering::Permission {
                holder: Holder { agent: agent.node.clone(), call },
                answer,
            },
        );
    let _wait = Wait {
        shard: shard.this(),
        conversation: conversation.clone(),
        request: request.clone(),
    };
    let checked = shard
        .store()
        .check_permission(conversation.clone(), request.clone(), needed, command, agent)
        .await
        .map_err(|error| RpcError::Failed(error.to_string()))?;
    let Checked::Raised { replaced } = checked else {
        return Ok(());
    };
    for older in &replaced {
        shard.waits().decided(older, Decided::Replaced);
    }
    changed(shard, &conversation);
    shard.asked(&conversation, request, ask);
    shard.raised().await;
    port.awaits_user(true).await?;
    match held(decided, port).await? {
        Decided::Allowed => Ok(()),
        Decided::Denied => Err(RpcError::Failed(format!(
            "the user denied this conversation permission to {action}; the command was not run"
        ))),
        Decided::Replaced => Err(RpcError::Failed(
            "a newer run of this command asks the user instead; this one was not run".into(),
        )),
    }
}

/// Holds a call until `decided` ends its wait.
///
/// Once allowed, the call no longer waits for the user. A call stopped
/// meanwhile is never passed on, even when the allow is ready at the same
/// moment: the stop is looked at last, after every wait, right before the
/// dispatch would run the handler.
async fn held(decided: oneshot::Receiver<Decided>, port: &RpcPort) -> Result<Decided, RpcError> {
    let stopped = || RpcError::Port(PortError::Ended("the call was stopped".into()));
    let decided = tokio::select! {
        biased;
        () = port.cancelled() => return Err(stopped()),
        // The sender goes only with a decision, or with the shard as the
        // backend stops.
        decided = decided => decided.map_err(|_| RpcError::Failed("the backend stopped while the call waited for the user's answer".into()))?,
    };
    if decided == Decided::Allowed {
        port.awaits_user(false).await?;
    }
    if port.is_cancelled() {
        return Err(stopped());
    }
    Ok(decided)
}

fn new_request_id() -> PermissionRequestId {
    PermissionRequestId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty")
}

/// What a grant of the category `id` lets the conversation do, as the
/// card's title, the refusal and the decision's message say it: its id when
/// no command declares it now.
async fn category_action(shard: &dyn PermissionShard, id: &str) -> String {
    declared(shard)
        .await
        .into_iter()
        .find(|category| category.id == id)
        .map_or_else(|| id.to_owned(), |category| category.action)
}

/// The conversation, the agent and the command line of a call that asks
/// the user.
struct Asking {
    conversation: ConversationId,
    agent: AskingAgent,
    /// The provider's id of the agent's `shell` call.
    call: String,
    command: String,
}

fn asking(shard: &dyn PermissionShard, invocation: &RpcInvocation) -> Result<Asking, RpcError> {
    let conversation = ConversationId::try_from(invocation.context.conversation.as_str())
        .map_err(|_| RpcError::Failed("the call names no conversation".into()))?;
    let caller = invocation
        .caller
        .as_ref()
        .ok_or_else(|| RpcError::Failed("a command that asks the user runs for an agent".into()))?;
    let agent = shard
        .asking_agent(&conversation, &caller.node)
        .map_err(RpcError::Failed)?;
    Ok(Asking {
        conversation,
        agent,
        call: caller.tool_use_id.clone(),
        command: command_line(invocation)?,
    })
}

/// Asks the user for the credential `name` and waits for the answer,
/// until `cancelled` resolves, which withdraws the request. A credential
/// of the name saved with other fields refuses the request before the
/// user sees it.
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
        ..
    } = asking(shard, invocation)?;
    let request = new_request_id();
    // Waiting before the request is stored, so an answer that comes at
    // once finds its command.
    let (answer, answered) = oneshot::channel();
    shard
        .waits()
        .wait(request.clone(), conversation.clone(), Answering::Credential(answer));
    let _wait = Wait {
        shard: shard.this(),
        conversation: conversation.clone(),
        request: request.clone(),
    };
    let asked = shard
        .store()
        .ask_credential(conversation.clone(), request.clone(), name.clone(), ask, command, agent)
        .await
        .map_err(|error| RpcError::Failed(error.to_string()))?;
    if let Asked::FieldsDiffer(fields) = asked {
        return Err(fields_differ(&name, &fields));
    }
    changed(shard, &conversation);
    shard.asked(&conversation, request, PushAsk::Credential { name });
    tokio::select! {
        // The sender goes only with the answer or a withdrawal.
        answer = answered => Ok(answer.unwrap_or(Answer::Withdrawn)),
        () = cancelled => Err(RpcError::Failed("the call was cancelled".into())),
    }
}

/// Saves the credential `name` from the `values` the call's agent piped,
/// kept as `ask` says, or new values of the saved one of that name, which
/// has the same fields.
pub async fn save_credential(
    shard: &dyn PermissionShard,
    invocation: &RpcInvocation,
    name: String,
    ask: CredentialAsk,
    values: BTreeMap<String, String>,
) -> Result<Answered, RpcError> {
    let Asking { conversation, .. } = asking(shard, invocation)?;
    let saved = shard
        .store()
        .save_credential(name.clone(), ask, values, shard.credential_seal())
        .await
        .map_err(|error| RpcError::Failed(error.to_string()))?;
    match saved {
        SavedByAgent::Saved(answered) => {
            changed(shard, &conversation);
            for other in &answered.others {
                changed(shard, other);
            }
            shard.credentials_changed();
            Ok(answered)
        }
        SavedByAgent::FieldsDiffer(fields) => Err(fields_differ(&name, &fields)),
        SavedByAgent::Unfit(problem) => Err(RpcError::Failed(problem.to_string())),
    }
}

/// The refusal of a credential whose name is saved with other `fields`.
fn fields_differ(name: &str, fields: &[CredentialField]) -> RpcError {
    let fields = fields
        .iter()
        .map(|field| format!("{}:{}", field.name, field.kind.as_str()))
        .collect::<Vec<_>>()
        .join(", ");
    RpcError::Failed(format!(
        "\u{201c}{name}\u{201d} is saved with other fields ({fields}): use those fields, or another name"
    ))
}

/// Answers the waiting commands of the conversation's credential requests,
/// which an archive withdrew.
pub fn withdrawn(shard: &dyn PermissionShard, conversation: &ConversationId) {
    let requests: Vec<PermissionRequestId> = shard
        .waits()
        .0
        .borrow()
        .iter()
        .filter(|(_, waiting)| {
            waiting.conversation == *conversation && matches!(waiting.answer, Answering::Credential(_))
        })
        .map(|(request, _)| request.clone())
        .collect();
    for request in requests {
        shard.waits().answer(&request, Answer::Withdrawn);
    }
}

/// The value of the field `field` of the credential `name`, for the call's
/// agent, whose conversation has the grant of the credentials.
pub async fn read_credential(
    shard: &dyn PermissionShard,
    invocation: &RpcInvocation,
    name: String,
    field: String,
) -> Result<String, RpcError> {
    asking(shard, invocation)?;
    let read = shard
        .store()
        .read_credential(name.clone(), field.clone(), shard.credential_seal())
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
        CredentialRead::Missing => Err(RpcError::Failed(format!(
            "\u{201c}{name}\u{201d}: the field \u{201c}{field}\u{201d} has no value yet; the user fills it in Settings \u{203a} Credentials"
        ))),
        CredentialRead::NoField(fields) => Err(RpcError::Failed(format!(
            "\u{201c}{name}\u{201d}: no field \u{201c}{field}\u{201d}; its fields are {}",
            fields
                .iter()
                .map(|field| field.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
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
) -> Result<ConversationPermissions, StorageError> {
    let revision = shard.revisions().of(conversation);
    let requests = shard.store().permission_requests(conversation.clone()).await?;
    let categories = declared(shard).await;
    let category = |id: &str| {
        let declared = categories.iter().find(|category| category.id == id);
        PermissionCategory {
            id: id.to_owned(),
            action: declared.map(|category| category.action.clone()),
            description: declared.map(|category| category.description.clone()),
        }
    };
    let mut listed = Vec::with_capacity(requests.len());
    for Undecided { request, renewal } in requests {
        let (id, command, agent, created_at) = (
            request.id,
            request.command,
            request.agent.subagent,
            request.created_at,
        );
        listed.push(match request.ask {
            StoredAsk::Permission(asked) => PermissionRequest::Permission {
                id,
                categories: vec![category(&asked)],
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
                renewal,
                command,
                agent,
                created_at,
            },
        });
    }
    Ok(ConversationPermissions {
        revision,
        requests: listed,
    })
}

/// Decides the undecided request `request`.
///
/// - An allow of a permission request grants its category to the conversation
///   and decides each request of that category.
/// - A deny decides this one alone.
///
/// The call held for each permission request decided goes on or fails, and each
/// agent that asked is sent its message after the answer.
///
/// A credential request is allowed with `credential`, its values, which saves
/// the credential for the request's keep. Its
/// command, which waits, returns with the answer instead of a message.
pub async fn decide(
    shard: &dyn PermissionShard,
    conversation: &ConversationId,
    request: PermissionRequestId,
    decision: PermissionDecision,
    credential: Option<RequestedValues>,
) -> Result<(), PermissionError> {
    match (decision, credential) {
        (PermissionDecision::Allow, Some(RequestedValues { values })) => {
            let Answered { credential, keep, others } = shard
                .store()
                .answer_credential(
                    conversation.clone(),
                    request.clone(),
                    values,
                    shard.credential_seal(),
                )
                .await??;
            changed(shard, conversation);
            for other in &others {
                changed(shard, other);
            }
            shard.credentials_changed();
            shard.waits().answer(
                &request,
                Answer::Saved { credential, keep },
            );
            Ok(())
        }
        (PermissionDecision::Deny, Some(_)) => Err(PermissionError::InvalidAnswer(
            AnswerProblem::DeclinedWithValues,
        )),
        (decision, None) => {
            let decided = shard
                .store()
                .decide_permission(conversation.clone(), request, decision)
                .await??;
            let outcome = match decision {
                PermissionDecision::Allow => PermissionOutcome::Allowed,
                PermissionDecision::Deny => PermissionOutcome::Denied,
            };
            settle(shard, conversation, decided, outcome);
            Ok(())
        }
    }
}

/// Tells the agents of the requests `decided` the decision `outcome`, and
/// every page that the conversation's requests changed.
fn settle(shard: &dyn PermissionShard, conversation: &ConversationId, decided: Vec<StoredRequest>, outcome: PermissionOutcome) {
    changed(shard, conversation);
    for request in decided {
        match request.ask {
            StoredAsk::Credential { .. } => {
                shard.waits().answer(&request.id, Answer::Declined);
                continue;
            }
            StoredAsk::Permission(_) => shard.waits().decided(
                &request.id,
                match outcome {
                    PermissionOutcome::Allowed | PermissionOutcome::Reached => Decided::Allowed,
                    PermissionOutcome::Denied => Decided::Denied,
                },
            ),
        }
        let this = shard.this();
        shard.spawn(Box::pin(async move { deliver(this, request).await }));
    }
}

/// The permission requests of `conversation` that hold a call `stops` names,
/// before the user's action stops it.
pub fn held_requests(shard: &dyn PermissionShard, conversation: &ConversationId, stops: &Stops<'_>) -> Vec<PermissionRequestId> {
    shard.waits().held_by(conversation, stops)
}

/// Denies the held requests `requests`, as Deny does: the card shows each
/// denied and its call returns as any call does, with the command's end.
///
/// A request whose call is gone, stopped by its parent or lost with a
/// restart, is not among the held ones and stays on its card.
pub async fn deny(
    shard: &dyn PermissionShard,
    conversation: &ConversationId,
    requests: Vec<PermissionRequestId>,
) -> Result<(), PermissionError> {
    for request in requests {
        match decide(shard, conversation, request, PermissionDecision::Deny, None).await {
            // Another decision took it first.
            Ok(()) | Err(PermissionError::NotFound) => {}
            Err(error) => return Err(error),
        }
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
    let category = match &request.ask {
        StoredAsk::Permission(category) => category.clone(),
        // A credential request's command returns its answer.
        StoredAsk::Credential { .. } => return,
    };
    let action = category_action(&*shard, &category).await;
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
            categories: vec![category.clone()],
            credential: None,
            host: None,
            pairing: false,
        },
        origin: None,
    };
    let admitted = shard
        .admit(&request.conversation, &request.agent.node, &message)
        .await;
    match admitted {
        Ok(_) => {
            if let Err(error) = shard.store().delivered_permission(request.id.clone()).await {
                // The request stays decided, and the next start delivers it
                // again under its id, which the agent admits once.
                tracing::warn!(event = "permission_decision.forget_failed", request = %request.id, conversation = %request.conversation, %error, "a delivered permission decision was not forgotten");
            }
        }
        Err(error) => {
            tracing::warn!(
                event = "permission_decision.delivery_failed",
                request = %request.id,
                conversation = %request.conversation,
                %error,
                retry = "next_start",
                "a permission decision was not delivered"
            );
        }
    }
}

/// The message the agent reads: the decision alone, since the call that
/// waited says itself whether it ran. A message delivered to an ancestor of
/// the closed subagent that asked names that subagent.
fn content(
    request: &StoredRequest,
    outcome: PermissionOutcome,
    action: &str,
    recipient: &NodeId,
) -> String {
    let mut text = match outcome {
        PermissionOutcome::Allowed => format!("The user allowed this conversation to {action}."),
        PermissionOutcome::Denied => format!("The user denied this conversation permission to {action}."),
        PermissionOutcome::Reached => {
            format!("Another conversation's agent reached this conversation, which allows it to {action}.")
        }
    };
    if recipient != &request.agent.node
        && let Some(subagent) = &request.agent.subagent
    {
        text.push_str(&format!(
            " Agent {} ({}) asked and has closed since.",
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
            tracing::warn!(event = "permission_categories.read_failed", %error, "the permission categories were not read");
            Vec::new()
        }
    }
}
