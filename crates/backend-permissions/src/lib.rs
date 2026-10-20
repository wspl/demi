//! Conversation permissions: the check every `rpc` call passes in the
//! backend's dispatch before its handler; the requests a call without a
//! grant raises, which the call waits on until the user decides; the user's
//! decisions, which end those waits, the grants an allow records, each kept
//! for the conversation's life, and the message each decision sends the
//! agent that asked, delivered again at start when a restart cut it off.
//! And the conversation's other requests, which share the list, the mark
//! and the routes: an agent's request for a credential, which its command
//! waits for, and a read of a credential the conversation may not use yet,
//! which fails and asks for an allowance.
//!
//! Every command source passes through the check, a plugin's, the product's
//! and the agent runtime's alike, so it belongs to none of them. What it
//! needs of its user's shard it reaches through [`PermissionShard`].
//!
//! Demi has no general permission system: an agent runs commands on its Host,
//! edits files and opens pages without asking, because the user chose the Host
//! and the directory. It asks only before acting on Demi itself, on what the
//! user owns beyond the conversation. Each such power is one broad category,
//! which the user judges once for a conversation; a host is allowed on its
//! own, one host at a time.
//!
//! ```text
//! demi skills add ... -> dispatch: the check -- grant? -- yes --> handler
//!                                      | no
//!                          request recorded, call held (a running command)
//!                                      | the user: Allow / Deny
//!             grant stored, message to the agent that asked, call goes on
//! ```
//!
//! A request records one refused call. A newer request of the same
//! categories from the same agent replaces an undecided one, since the
//! agent ran the command again and its newest line is the one to show.
//! Allow records the grant of each of the request's categories and decides
//! as allowed every request of the conversation whose categories are now
//! granted; Deny decides that one request and records nothing, so the next
//! attempt raises a new request. Requests never expire: they survive a
//! restart, and archiving withdraws them. A grant covers every agent of the
//! conversation's tree, those spawned later included, lasts the
//! conversation's life, and survives an archive; a Fork copies none, since
//! the user decides for a new conversation again.
//!
//! The waiting command is an ordinary running command: its window, the turn,
//! Stop and the Host's idle rules are those of any command. It sends no
//! progress report, whatever its interval, since nothing about it changes
//! until the user answers and each report would wake the agent for nothing;
//! its interval counts again from the end of the wait. A call that ends
//! before the decision, stopped or lost with a restart, leaves its request
//! as it is: an Allow still grants the category for the commands that come
//! later.

use std::cell::RefCell;
use std::collections::HashMap;
use std::future::Future;
use std::rc::Rc;
use std::sync::Arc;

use demi_shared_types::Timestamp;
use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::credentials::{CredentialSeal, StoredCredential};
use demi_backend_database::permissions::{
    AnswerProblem, Answered, AskedHost, Asked, AskingAgent, Checked, CredentialAsk, CredentialRead,
    Needed, PermissionRefusal, StoredAsk, StoredRequest, Undecided,
};
use demi_backend_page_sync::{Part, Revisions, UserMarks};
use demi_command_declarations::Category;
use demi_host_interface::{PortError, RpcError, RpcInvocation, RpcPort};
use demi_shared_types::{AgentMessage, AgentMessageEvent, BlockId, NodeId, PermissionOutcome};
use demi_web_api_protocol::credentials::{Keep, RequestedValues};
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use demi_web_api_protocol::permissions::{
    ConversationPermissions, PermissionCategory, PermissionDecision, PermissionHost,
    PermissionRequest, PermissionRequestId,
};
use demi_web_api_protocol::push::PushAsk;
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

    /// The calls that wait for the answers to their requests.
    fn waits(&self) -> &Waits;

    /// What seals and opens the values of the user's credentials.
    fn credential_seal(&self) -> Arc<dyn CredentialSeal>;

    /// The user's credentials changed: every page receives them, and the
    /// expiry looks at their times again.
    fn credentials_changed(&self);

    /// The name the user knows the device by: Cloud for the user's Cloud,
    /// a paired device's own name.
    fn host_name<'a>(&'a self, device: &'a DeviceId) -> LocalBoxFuture<'a, String>;

    /// When the message of a run a URL started came, while every turn of
    /// the conversation works for it, until the user next writes to it: each
    /// credential read since asks. None while the user's own message is the
    /// latest.
    fn url_message_since<'a>(&'a self, conversation: &'a ConversationId) -> LocalBoxFuture<'a, Option<Timestamp>>;

    /// The conversation asks the user what `ask` says, by the request
    /// `request`, which the user's browsers are notified of.
    fn asked(&self, conversation: &ConversationId, request: PermissionRequestId, ask: PushAsk);

    /// A call's request is stored and its pages told, before the call
    /// waits for the answer: where a test holds the check, to decide the
    /// request meanwhile. Nothing happens here otherwise.
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
    Permission(oneshot::Sender<Decided>),
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

    /// Ends the wait of the call held for the permission request `request`.
    fn decided(&self, request: &PermissionRequestId, decided: Decided) {
        if let Some(Answering::Permission(sender)) = self.take(request) {
            // A call that stopped waiting leaves its request as it is.
            let _ = sender.send(decided);
        }
    }

    fn take(&self, request: &PermissionRequestId) -> Option<Answering> {
        self.0.borrow_mut().remove(request).map(|waiting| waiting.answer)
    }
}

/// A call's wait for the answer to its request, which ends its entry in
/// [`Waits`] however the call ends. A credential request that ends without
/// an answer, whether its command was stopped or its handler dropped, is
/// withdrawn, since nobody waits for it; a permission request stays for the
/// user to decide.
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
            match shard.control().withdraw_credential_request(request.clone()).await {
                Ok(true) => changed(&*shard, &conversation),
                Ok(false) => {}
                // The next start withdraws every credential request left.
                Err(error) => tracing::warn!(%request, "a credential request was not withdrawn: {error}"),
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
    #[error("\u{201c}{0}\u{201d} was removed, so this conversation can\u{2019}t use it.")]
    Removed(String),
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
            PermissionRefusal::Removed(name) => Self::Removed(name),
        }
    }
}

/// What one `rpc` call needs of its conversation: its leaf's category, or
/// the host it brings in; no command needs both.
pub enum Need<'a> {
    Category(&'a Category),
    Host(AskedHost),
}

/// The check of one `rpc` call that needs `need`: it passes when the call's
/// conversation has the grant of the category or may reach the host,
/// whichever agent of its tree runs it. Otherwise it records the request
/// and holds the call until the user decides, with the waiting line on its
/// job, while no handler runs: an allow passes it, and a deny or a newer
/// request of the same agent and need fails it with the line the agent
/// reads. A call stopped meanwhile fails, an allow ready at the same moment
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
        command,
    } = asking(shard, invocation)?;
    let (needed, action, ask) = match need {
        Need::Category(category) => {
            let ask = PushAsk::Permission {
                categories: vec![PermissionCategory {
                    id: category.id.clone(),
                    action: Some(category.action.clone()),
                    description: None,
                }],
                host: None,
            };
            (Needed::Category(category.id.clone()), category.action.clone(), ask)
        }
        Need::Host(host) => {
            let action = host_action(shard, &host).await;
            let shown = match &host {
                AskedHost::Device(device) => PermissionHost::Device {
                    device_id: device.clone(),
                    name: shard.host_name(device).await,
                },
                AskedHost::Pairing(_) => PermissionHost::Pairing {},
            };
            let ask = PushAsk::Permission {
                categories: Vec::new(),
                host: Some(shown),
            };
            (Needed::Host(host), action, ask)
        }
    };
    // The pass an allow of a pairing gives, which the call it lets through
    // uses up.
    let pairing = match &needed {
        Needed::Host(AskedHost::Pairing(code)) => Some(code.clone()),
        Needed::Category(_) | Needed::Host(AskedHost::Device(_)) => None,
    };
    let request = new_request_id();
    // Waiting before the request is stored, so that an allow of another
    // request that grants its need finds the call once the request is
    // there, whatever the order of the two transactions.
    let (answer, decided) = oneshot::channel();
    shard
        .waits()
        .wait(request.clone(), conversation.clone(), Answering::Permission(answer));
    let _wait = Wait {
        shard: shard.this(),
        conversation: conversation.clone(),
        request: request.clone(),
    };
    let checked = shard
        .control()
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
    port.awaits_user(Some(&format!(
        "waiting for the user's permission to {action}; its end will report to you, so do not run it again"
    )))
    .await?;
    let passed = async {
        if let Some(code) = pairing {
            shard
                .control()
                .use_pairing_pass(conversation.clone(), code)
                .await
                .map_err(|error| RpcError::Failed(error.to_string()))?;
        }
        Ok(())
    };
    match held(decided, port, passed).await? {
        Decided::Allowed => Ok(()),
        Decided::Denied => Err(RpcError::Failed(format!(
            "the user denied this conversation permission to {action}; the command was not run"
        ))),
        Decided::Replaced => Err(RpcError::Failed(
            "a newer run of this command asks the user instead; this one was not run".into(),
        )),
    }
}

/// Holds a call until `decided` ends its wait; once allowed, takes its
/// waiting line off its job and does `passed`, what passing it uses up. A
/// call stopped meanwhile is never passed on, even when the allow is ready
/// at the same moment: the stop is looked at last, after every wait, right
/// before the dispatch would run the handler.
async fn held(
    decided: oneshot::Receiver<Decided>,
    port: &RpcPort,
    passed: impl Future<Output = Result<(), RpcError>>,
) -> Result<Decided, RpcError> {
    let stopped = || RpcError::Port(PortError::Ended("the call was stopped".into()));
    let decided = tokio::select! {
        biased;
        () = port.cancelled() => return Err(stopped()),
        // The sender goes only with a decision, or with the shard as the
        // backend stops.
        decided = decided => decided.map_err(|_| RpcError::Failed("the backend stopped while the call waited for the user's answer".into()))?,
    };
    if decided == Decided::Allowed {
        port.awaits_user(None).await?;
        passed.await?;
    }
    if port.is_cancelled() {
        return Err(stopped());
    }
    Ok(decided)
}

fn new_request_id() -> PermissionRequestId {
    PermissionRequestId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty")
}

/// What reaching `host` lets the conversation do, as the card's title, the
/// refusal and the decision's message say it.
async fn host_action(shard: &dyn PermissionShard, host: &AskedHost) -> String {
    match host {
        AskedHost::Device(device) => format!("reach {}", shard.host_name(device).await),
        AskedHost::Pairing(_) => "pair a computer and reach it".to_owned(),
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
        .control()
        .ask_credential(conversation.clone(), request.clone(), name.clone(), ask, command, agent)
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
    shard.asked(&conversation, request, PushAsk::Credential { name });
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
        .filter(|(_, waiting)| {
            waiting.conversation == *conversation && matches!(waiting.answer, Answering::Credential(_))
        })
        .map(|(request, _)| request.clone())
        .collect();
    for request in requests {
        shard.waits().answer(&request, Answer::Withdrawn);
    }
}

/// The field `field` of the credential `name`, for the call's agent: its
/// value when the conversation may use the credential. Otherwise it records
/// the allowance request and fails the call as a refused permission does.
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
    let asks_since = shard.url_message_since(&conversation).await;
    let read = shard
        .control()
        .read_credential(
            conversation.clone(),
            name.clone(),
            field.clone(),
            command,
            agent,
            asks_since,
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
        CredentialRead::Raised { id, name: saved } => {
            changed(shard, &conversation);
            let action = use_action(&saved);
            shard.asked(&conversation, id, PushAsk::Allowance { name: saved });
            Err(RpcError::Failed(format!(
                "this conversation needs the user's permission to {action}; the request was sent to the user, and you will be told when the user decides"
            )))
        }
    }
}

/// What an allowance lets the conversation do, as the card's title, the
/// refusal and the decision's message say it.
fn use_action(name: &str) -> String {
    format!("use the credential \u{201c}{name}\u{201d}")
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
    let mut listed = Vec::with_capacity(requests.len());
    for Undecided { request, renewal } in requests {
        let (id, command, agent, created_at) = (
            request.id,
            request.command,
            request.agent.subagent,
            request.created_at,
        );
        listed.push(match request.ask {
            StoredAsk::Permission(needed) => PermissionRequest::Permission {
                id,
                categories: match &needed {
                    Needed::Category(id) => vec![category(id)],
                    Needed::Host(_) => Vec::new(),
                },
                host: match needed {
                    Needed::Host(AskedHost::Device(device)) => Some(PermissionHost::Device {
                        name: shard.host_name(&device).await,
                        device_id: device,
                    }),
                    Needed::Host(AskedHost::Pairing(_)) => Some(PermissionHost::Pairing {}),
                    Needed::Category(_) => None,
                },
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
            StoredAsk::Allowance { name, credential } => PermissionRequest::Allowance {
                id,
                name,
                removed: credential.is_none(),
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

/// Decides the undecided request `request`: an allow of a permission
/// request grants its categories to the conversation and decides each
/// request whose categories are then all granted, an allow of an allowance
/// request allows its credential and decides each request to use it, a
/// deny decides this one alone. The call held for each permission request
/// decided goes on or fails, and each agent that asked is sent its message
/// after the answer. A credential request is allowed with `credential`,
/// its values, which saves the credential for the request's keep and
/// allows it in the conversation; its command, which waits, returns with
/// the answer instead of a message.
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
                .control()
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
                match request.ask {
                    StoredAsk::Credential { .. } => {
                        shard.waits().answer(&request.id, Answer::Declined);
                        continue;
                    }
                    StoredAsk::Permission(_) => shard.waits().decided(
                        &request.id,
                        match outcome {
                            PermissionOutcome::Allowed => Decided::Allowed,
                            PermissionOutcome::Denied => Decided::Denied,
                        },
                    ),
                    StoredAsk::Allowance { .. } => {}
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
    let mut host_name = None;
    let action = match &request.ask {
        StoredAsk::Permission(Needed::Category(id)) => declared(&*shard)
            .await
            .into_iter()
            .find(|category| category.id == *id)
            .map_or_else(|| id.clone(), |category| category.action),
        StoredAsk::Permission(Needed::Host(host)) => {
            if let AskedHost::Device(device) = host {
                host_name = Some(shard.host_name(device).await);
            }
            host_action(&*shard, host).await
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
            categories: match &request.ask {
                StoredAsk::Permission(Needed::Category(id)) => vec![id.clone()],
                StoredAsk::Permission(Needed::Host(_)) | StoredAsk::Allowance { .. } | StoredAsk::Credential { .. } => {
                    Vec::new()
                }
            },
            credential: match &request.ask {
                StoredAsk::Allowance { name, .. } => Some(name.clone()),
                StoredAsk::Permission { .. } | StoredAsk::Credential { .. } => None,
            },
            host: host_name.clone(),
            pairing: matches!(
                request.ask,
                StoredAsk::Permission(Needed::Host(AskedHost::Pairing(_)))
            ),
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

/// The message the agent reads: the decision alone for a permission
/// request, whose call that waited says itself whether it ran, and with the
/// command line for the use of a credential, whose read failed. One
/// delivered to an ancestor of the closed subagent that asked names it.
fn content(
    request: &StoredRequest,
    outcome: PermissionOutcome,
    action: &str,
    recipient: &NodeId,
) -> String {
    let command = &request.command;
    let mut text = match (&request.ask, outcome) {
        (StoredAsk::Allowance { .. }, PermissionOutcome::Allowed) => format!(
            "The user allowed this conversation to {action}; the command `{command}` can now run."
        ),
        (StoredAsk::Allowance { .. }, PermissionOutcome::Denied) => format!(
            "The user denied this conversation permission to {action}; the command `{command}` was not run."
        ),
        (_, PermissionOutcome::Allowed) => format!("The user allowed this conversation to {action}."),
        (_, PermissionOutcome::Denied) => {
            format!("The user denied this conversation permission to {action}.")
        }
    };
    if recipient != &request.agent.node
        && let Some(subagent) = &request.agent.subagent
    {
        let did = match request.ask {
            // Its message names the command the subagent ran.
            StoredAsk::Allowance { .. } => "ran it",
            _ => "asked",
        };
        text.push_str(&format!(
            " Agent {} ({}) {did} and has closed since.",
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

#[cfg(test)]
mod tests {
    use demi_host_interface::testing::MemoryPort;
    use tokio_util::sync::CancellationToken;

    use super::*;

    // The user stops a held call and allows its request at the same moment:
    // both are ready before the call looks again, and the call fails
    // stopped, so the dispatch never runs its handler. A scenario cannot
    // order the two; here both are settled before the hold is polled.
    // Instant.
    #[tokio::test]
    async fn a_call_stopped_as_its_request_is_allowed_is_not_passed_on() {
        let stop = CancellationToken::new();
        let port = MemoryPort::new().port(stop.clone());
        let (answer, decided) = oneshot::channel();
        answer.send(Decided::Allowed).unwrap();
        stop.cancel();
        let held = held(decided, &port, async { Ok(()) }).await;
        assert!(matches!(held, Err(RpcError::Port(PortError::Ended(_)))), "{held:?}");
    }
}
