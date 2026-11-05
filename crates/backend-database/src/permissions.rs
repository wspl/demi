//! A conversation's requests, and what the decisions record.
//!
//! # Requests
//!
//! The requests are:
//!
//! - the permission requests refused commands raise;
//! - the credential requests of agents that wait for the user to save a
//!   credential.
//!
//! Each is undecided until the user decides it, and a decided one is kept until
//! its message is in the asking agent's checkpoint.
//!
//! A permission request asks for one category, which covers a whole
//! capability: every object and every step of it. A call that brings in a
//! device the conversation does not reach without it asks for [`DEVICES`].
//!
//! # Decisions
//!
//! The decisions record the grants, one per conversation and category, each
//! kept for the conversation's life.
//!
//! Each change reads and writes in one transaction, so a check and a decision
//! never interleave.

use std::collections::BTreeMap;

use demi_shared_types::{NodeId, PermissionOutcome, Timestamp};
use demi_web_api_protocol::credentials::{CredentialField, Keep};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserId};
use demi_web_api_protocol::permissions::{PermissionDecision, PermissionRequestId, RequestingAgent};
use garde::Validate;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::StorageError;
use super::columns::{decode, instant, json, to_json};
use super::control::ControlService;
use super::conversation_index::reaches_without_grant;
use super::credentials::{
    self, CredentialSeal, NewCredential, StoredCredential, values_of,
};
use super::drafts::archived;

const REQUESTS: &str = "permission_requests";

const REQUEST_COLUMNS: &str = "id, conversation_id, kind, categories, credential_name, ask, command, node_id, agent_number, agent_description, created_at, decision, decided_at";

/// The category of the user's devices: bringing any of them into a
/// conversation, pairing a computer and port forwards.
pub const DEVICES: &str = "host.use";

/// A request's categories as its column holds them: the one it asks for.
#[derive(Debug, Serialize, Deserialize, Validate)]
#[garde(transparent)]
struct Categories(#[garde(inner(length(min = 1)))] Vec<String>);

/// What a call needs and its conversation may lack: one category's grant,
/// or one device to reach, which [`DEVICES`] covers; no command needs both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Needed {
    Category(String),
    Device(DeviceId),
}

impl Needed {
    /// The category a request for it asks.
    pub fn category(&self) -> &str {
        match self {
            Self::Category(category) => category,
            Self::Device(_) => DEVICES,
        }
    }
}

/// The agent that ran a refused command: its node, and, for a subagent, the
/// number and description the model knows it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskingAgent {
    pub node: NodeId,
    /// None for the root.
    pub subagent: Option<RequestingAgent>,
}

/// What a credential request asks for beside the name, as its `ask` column
/// holds it: credential request format 1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CredentialAsk {
    /// Why the agent asks; empty for no reason.
    #[garde(skip)]
    pub reason: String,
    #[garde(length(min = 1), dive)]
    pub fields: Vec<CredentialField>,
    /// How long the credential is kept once saved, as the agent decided.
    #[garde(skip)]
    pub keep: Keep,
}

/// What a request asks of the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredAsk {
    /// The category the call lacked.
    Permission(String),
    /// A credential of this name to save.
    Credential { name: String, ask: CredentialAsk },
}

/// A request as it is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRequest {
    pub id: PermissionRequestId,
    pub conversation: ConversationId,
    pub ask: StoredAsk,
    /// The command line as the agent ran it.
    pub command: String,
    pub agent: AskingAgent,
    pub created_at: Timestamp,
    /// None while the user has not decided.
    pub decision: Option<Decision>,
}

/// How and when the user decided a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub outcome: PermissionOutcome,
    pub at: Timestamp,
}

/// What the check finds for a call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Checked {
    /// The conversation has every grant: the call is dispatched.
    Granted,
    /// The grant is missing: the request was recorded, in place of the
    /// undecided ones of the same category from the same agent, `replaced`.
    Raised { replaced: Vec<PermissionRequestId> },
}

/// Why a decision changed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionRefusal {
    /// The conversation is archived.
    Archived,
    /// No undecided request of that id in the conversation.
    NotFound,
    /// The answer does not fit the request.
    InvalidAnswer(AnswerProblem),
    /// A credential of the requested name is saved with other fields, as
    /// the words for the user say.
    FieldsDiffer(String),
}

/// What answering a credential request saved, with the other conversations
/// whose credential requests of its name wait, whose renewal follows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answered {
    pub credential: StoredCredential,
    /// How long it is kept: the request's keep.
    pub keep: Keep,
    pub others: Vec<ConversationId>,
}

/// An undecided request as a page lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Undecided {
    pub request: StoredRequest,
    /// For a credential request, whether the user has a credential of its
    /// name, whatever its case, whose values it asks anew.
    pub renewal: bool,
}

/// What asking for a credential found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Asked {
    /// The request waits for the user.
    Waiting,
    /// A credential of the name is saved with these other fields.
    FieldsDiffer(Vec<CredentialField>),
}

/// What an agent's save of a credential from its values did.
#[derive(Debug)]
pub enum SavedByAgent {
    /// The credential, new or with new values.
    Saved(Answered),
    /// A credential of the name is saved with these other fields.
    FieldsDiffer(Vec<CredentialField>),
    /// The values do not fit the fields.
    Unfit(AnswerProblem),
}

/// What a read of a credential's field found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialRead {
    /// The field's value; the credential's last use is now.
    Value(String),
    /// The user has no credential of that name.
    NoCredential,
    /// The credential has no field of that name, but these.
    NoField(Vec<CredentialField>),
    /// The field has no value yet: the user fills it in Settings.
    Missing,
}

/// A decided request whose message the backend still holds, with the owner
/// of its conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Undelivered {
    pub owner: UserId,
    pub request: StoredRequest,
}

impl ControlService {
    /// The check of one call: whether the conversation has what `needed`
    /// names.
    ///
    /// When it lacks it, the check records the request `id` for `command` of
    /// the need's category, which replaces the undecided requests of that
    /// category from the same agent.
    pub async fn check_permission(
        &self,
        conversation: ConversationId,
        id: PermissionRequestId,
        needed: Needed,
        command: String,
        agent: AskingAgent,
    ) -> Result<Checked, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let reached = match &needed {
                Needed::Device(device) => reaches_without_grant(&transaction, &conversation, device)?,
                Needed::Category(_) => false,
            };
            if reached || granted_at(&transaction, &conversation, needed.category())?.is_some() {
                return Ok(Checked::Granted);
            }
            let stored = to_json(&[needed.category()]);
            let replaced = transaction
                .prepare_cached(
                    "DELETE FROM permission_requests
                     WHERE conversation_id = ?1 AND kind = 'permission' AND categories = ?2 AND node_id = ?3
                       AND decision IS NULL
                     RETURNING id",
                )?
                .query_map(
                    params![conversation.as_str(), stored, agent.node.as_str()],
                    |row| row.get::<_, String>(0),
                )?
                .map(|id| {
                    decode(REQUESTS, "id", PermissionRequestId::try_from(id?))
                })
                .collect::<Result<Vec<_>, StorageError>>()?;
            insert_request(
                &transaction,
                &id,
                &conversation,
                &Columns {
                    kind: "permission",
                    categories: Some(stored),
                    ..Columns::default()
                },
                &command,
                &agent,
                now,
            )?;
            transaction.commit()?;
            Ok(Checked::Raised { replaced })
        })
        .await
    }

    /// The conversation's undecided requests, oldest first, each credential
    /// request with whether it renews a saved credential.
    pub async fn permission_requests(
        &self,
        conversation: ConversationId,
    ) -> Result<Vec<Undecided>, StorageError> {
        self.call(move |connection, _| {
            let requests = connection
                .prepare_cached(&format!(
                    "SELECT {REQUEST_COLUMNS} FROM permission_requests
                     WHERE conversation_id = ?1 AND decision IS NULL ORDER BY created_at, rowid"
                ))?
                .query_map([conversation.as_str()], |row| Ok(request_row(row)))?
                .map(|row| row?)
                .collect::<Result<Vec<_>, StorageError>>()?;
            // Only a credential request needs the owner's credentials.
            let owner = if requests
                .iter()
                .any(|request| matches!(request.ask, StoredAsk::Credential { .. }))
            {
                Some(owner_of(connection, &conversation)?)
            } else {
                None
            };
            let mut listed = Vec::with_capacity(requests.len());
            for request in requests {
                let renewal = match (&request.ask, &owner) {
                    (StoredAsk::Credential { name, .. }, Some(owner)) => {
                        credentials::named(connection, owner, name)?.is_some()
                    }
                    _ => false,
                };
                listed.push(Undecided { request, renewal });
            }
            Ok(listed)
        })
        .await
    }

    /// Decides the undecided request `request`.
    ///
    /// - An allow of a permission request records the grant of its category. It
    ///   decides the request with every undecided permission request of the
    ///   conversation for that category.
    /// - A deny decides the request alone, and forgets a credential request, whose
    ///   agent waits for the answer rather than a message.
    ///
    /// A credential request is allowed only with its values
    /// ([`Self::answer_credential`]).
    ///
    /// Answers the requests it decided, whose messages the backend delivers next,
    /// and the credential request it denied.
    pub async fn decide_permission(
        &self,
        conversation: ConversationId,
        request: PermissionRequestId,
        decision: PermissionDecision,
    ) -> Result<Result<Vec<StoredRequest>, PermissionRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let decided = match undecided(&transaction, &conversation, &request)? {
                Err(refusal) => return Ok(Err(refusal)),
                Ok(stored) => match (&stored.ask, decision) {
                    (StoredAsk::Credential { .. }, PermissionDecision::Allow) => {
                        return Ok(Err(PermissionRefusal::InvalidAnswer(
                            AnswerProblem::ValuesMissing,
                        )));
                    }
                    (StoredAsk::Credential { .. }, PermissionDecision::Deny) => {
                        transaction.execute(
                            "DELETE FROM permission_requests WHERE id = ?1",
                            [request.as_str()],
                        )?;
                        transaction.commit()?;
                        return Ok(Ok(vec![stored]));
                    }
                    (StoredAsk::Permission(category), PermissionDecision::Allow) => {
                        let category = category.clone();
                        let requests = grant(&transaction, &conversation, &category, now)?;
                        transaction.commit()?;
                        return Ok(Ok(requests));
                    }
                    (StoredAsk::Permission(_), PermissionDecision::Deny) => vec![request.as_str().to_owned()],
                },
            };
            let requests = decide(&transaction, decided, PermissionOutcome::Denied, now)?;
            transaction.commit()?;
            Ok(Ok(requests))
        })
        .await
    }

    /// Records a credential request of the agent `agent` as `id`, unless
    /// the user has a credential of the name with other fields.
    pub async fn ask_credential(
        &self,
        conversation: ConversationId,
        id: PermissionRequestId,
        name: String,
        ask: CredentialAsk,
        command: String,
        agent: AskingAgent,
    ) -> Result<Asked, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let owner = owner_of(&transaction, &conversation)?;
            if let Some(saved) = credentials::named(&transaction, &owner, &name)?
                && !saved.same_fields(&ask.fields)
            {
                return Ok(Asked::FieldsDiffer(saved.fields));
            }
            insert_request(
                &transaction,
                &id,
                &conversation,
                &Columns {
                    kind: "credential",
                    credential_name: Some(name),
                    ask: Some(to_json(&ask)),
                    ..Columns::default()
                },
                &command,
                &agent,
                now,
            )?;
            transaction.commit()?;
            Ok(Asked::Waiting)
        })
        .await
    }

    /// Answers the credential request `request` with `values`, kept as the
    /// request's `keep` says, in one transaction.
    ///
    /// It saves a new credential of the request's name, or new values of the saved
    /// one with the same fields, and forgets the request, whose agent waits for
    /// the answer.
    pub async fn answer_credential(
        &self,
        conversation: ConversationId,
        request: PermissionRequestId,
        values: BTreeMap<String, String>,
        seal: std::sync::Arc<dyn CredentialSeal>,
    ) -> Result<Result<Answered, PermissionRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let stored = match undecided(&transaction, &conversation, &request)? {
                Ok(stored) => stored,
                Err(refusal) => return Ok(Err(refusal)),
            };
            let StoredAsk::Credential { name, ask } = &stored.ask else {
                return Ok(Err(PermissionRefusal::InvalidAnswer(
                    AnswerProblem::NotACredentialRequest,
                )));
            };
            if let Err(problem) = fitting(&ask.fields, &values) {
                return Ok(Err(PermissionRefusal::InvalidAnswer(problem)));
            }
            let answered = match store(&transaction, &conversation, name, ask, values, now, &*seal)? {
                Ok(answered) => answered,
                Err(_) => {
                    return Ok(Err(PermissionRefusal::FieldsDiffer(format!(
                        "\u{201c}{name}\u{201d} already has other fields; answer with the same fields or choose another name."
                    ))));
                }
            };
            transaction.execute("DELETE FROM permission_requests WHERE id = ?1", [request.as_str()])?;
            transaction.commit()?;
            Ok(Ok(answered))
        })
        .await
    }

    /// Saves the credential `name` an agent of the conversation gave
    /// `values` for, or new values of the saved one of that name with the
    /// same fields, kept as `ask` says, in one transaction.
    pub async fn save_credential(
        &self,
        conversation: ConversationId,
        name: String,
        ask: CredentialAsk,
        values: BTreeMap<String, String>,
        seal: std::sync::Arc<dyn CredentialSeal>,
    ) -> Result<SavedByAgent, StorageError> {
        self.call(move |connection, now| {
            if let Err(problem) = fitting(&ask.fields, &values) {
                return Ok(SavedByAgent::Unfit(problem));
            }
            let transaction = connection.transaction()?;
            Ok(match store(&transaction, &conversation, &name, &ask, values, now, &*seal)? {
                Ok(answered) => {
                    transaction.commit()?;
                    SavedByAgent::Saved(answered)
                }
                Err(fields) => SavedByAgent::FieldsDiffer(fields),
            })
        })
        .await
    }

    /// Forgets the undecided credential request `request`, whose command no
    /// longer waits for it. Answers whether it was there.
    pub async fn withdraw_credential_request(
        &self,
        request: PermissionRequestId,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let removed = connection.execute(
                "DELETE FROM permission_requests
                 WHERE id = ?1 AND kind = 'credential' AND decision IS NULL",
                [request.as_str()],
            )?;
            Ok(removed > 0)
        })
        .await
    }

    /// Forgets every credential request, as the backend starts: the
    /// commands that waited for them ended with the run before.
    pub async fn withdraw_credential_requests(&self) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute("DELETE FROM permission_requests WHERE kind = 'credential'", [])?;
            Ok(())
        })
        .await
    }

    /// The field `field` of the user's credential `name`, for the agent `agent` of
    /// the conversation.
    ///
    /// Its value, read now as the credential's last use.
    pub async fn read_credential(
        &self,
        conversation: ConversationId,
        name: String,
        field: String,
        seal: std::sync::Arc<dyn CredentialSeal>,
    ) -> Result<CredentialRead, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let owner = owner_of(&transaction, &conversation)?;
            let Some(stored) = credentials::named(&transaction, &owner, &name)? else {
                return Ok(CredentialRead::NoCredential);
            };
            if !stored.fields.iter().any(|known| known.name == field) {
                return Ok(CredentialRead::NoField(stored.fields));
            }
            let mut values = values_of(&transaction, &stored, &*seal)?;
            let Some(value) = values.remove(&field) else {
                return Ok(CredentialRead::Missing);
            };
            transaction.execute(
                "UPDATE user_credentials SET last_used_at = ?2 WHERE id = ?1",
                params![stored.id.as_str(), now.as_millisecond()],
            )?;
            transaction.commit()?;
            Ok(CredentialRead::Value(value))
        })
        .await
    }

    /// Forgets a decided request once its message is in the agent's
    /// checkpoint.
    pub async fn delivered_permission(
        &self,
        request: PermissionRequestId,
    ) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            connection.execute(
                "DELETE FROM permission_requests WHERE id = ?1 AND decision IS NOT NULL",
                [request.as_str()],
            )?;
            Ok(())
        })
        .await
    }

    /// Every decided request whose message was not delivered, oldest first,
    /// which the backend delivers when it starts.
    pub async fn undelivered_permissions(&self) -> Result<Vec<Undelivered>, StorageError> {
        self.call(move |connection, _| {
            let columns = REQUEST_COLUMNS
                .split(", ")
                .map(|column| format!("r.{column}"))
                .collect::<Vec<_>>()
                .join(", ");
            connection
                .prepare_cached(&format!(
                    "SELECT {columns}, c.user_id FROM permission_requests r
                     JOIN conversations c ON c.id = r.conversation_id
                     WHERE r.decision IS NOT NULL ORDER BY r.created_at, r.rowid"
                ))?
                .query_map([], |row| {
                    Ok((|| {
                        Ok(Undelivered {
                            owner: decode(
                                "conversations",
                                "user_id",
                                UserId::try_from(row.get::<_, String>("user_id")?),
                            )?,
                            request: request_row(row)?,
                        })
                    })())
                })?
                .map(|row| row?)
                .collect()
        })
        .await
    }
}

/// The columns of a request that differ by its kind.
#[derive(Default)]
struct Columns {
    kind: &'static str,
    categories: Option<String>,
    credential_name: Option<String>,
    ask: Option<String>,
}

fn insert_request(
    connection: &Connection,
    id: &PermissionRequestId,
    conversation: &ConversationId,
    columns: &Columns,
    command: &str,
    agent: &AskingAgent,
    now: Timestamp,
) -> Result<(), StorageError> {
    let (number, description) = match &agent.subagent {
        Some(subagent) => (
            Some(i64::try_from(subagent.number).expect("an agent number fits")),
            Some(subagent.description.as_str()),
        ),
        None => (None, None),
    };
    connection.execute(
        "INSERT INTO permission_requests
         (id, conversation_id, kind, categories, credential_name, ask, command,
          node_id, agent_number, agent_description, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            id.as_str(),
            conversation.as_str(),
            columns.kind,
            columns.categories,
            columns.credential_name,
            columns.ask,
            command,
            agent.node.as_str(),
            number,
            description,
            now.as_millisecond()
        ],
    )?;
    Ok(())
}

/// The undecided request `request` of the conversation, or why there is
/// none to decide.
fn undecided(
    connection: &Connection,
    conversation: &ConversationId,
    request: &PermissionRequestId,
) -> Result<Result<StoredRequest, PermissionRefusal>, StorageError> {
    if archived(connection, conversation)? {
        return Ok(Err(PermissionRefusal::Archived));
    }
    let stored = connection
        .query_row(
            &format!(
                "SELECT {REQUEST_COLUMNS} FROM permission_requests
                 WHERE id = ?1 AND conversation_id = ?2 AND decision IS NULL"
            ),
            params![request.as_str(), conversation.as_str()],
            |row| Ok(request_row(row)),
        )
        .optional()?
        .transpose()?;
    Ok(stored.ok_or(PermissionRefusal::NotFound))
}

/// The user who owns the conversation.
fn owner_of(connection: &Connection, conversation: &ConversationId) -> Result<UserId, StorageError> {
    let owner: String = connection.query_row(
        "SELECT user_id FROM conversations WHERE id = ?1",
        [conversation.as_str()],
        |row| row.get(0),
    )?;
    decode("conversations", "user_id", UserId::try_from(owner))
}

/// Why an answer does not fit its request: its English is the refusal's
/// message, and the field and the limit it names are the values the page
/// words it with.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnswerProblem {
    /// An allow of a credential request came without its values.
    #[error("Enter a value for each field to save the credential.")]
    ValuesMissing,
    /// The field labelled `label` has no value.
    #[error("Enter a value for \u{201c}{label}\u{201d}.")]
    ValueMissing { label: String },
    /// The answer has a value for `name`, which the request does not ask.
    #[error("The request has no field \u{201c}{name}\u{201d}.")]
    UnknownField { name: String },
    /// Values answer a request that is not a credential's.
    #[error("Only a credential request is answered with values.")]
    NotACredentialRequest,
    /// A declined request came with values.
    #[error("A declined request takes no values.")]
    DeclinedWithValues,
}

/// Saves the credential `name` with `values`, or new values of the saved one
/// of that name, kept as `ask` says. A saved one with other fields is left
/// as it is, and its fields answered.
fn store(
    connection: &Connection,
    conversation: &ConversationId,
    name: &str,
    ask: &CredentialAsk,
    values: BTreeMap<String, String>,
    now: Timestamp,
    seal: &dyn CredentialSeal,
) -> Result<Result<Answered, Vec<CredentialField>>, StorageError> {
    let owner = owner_of(connection, conversation)?;
    let keep_until = credentials::keep_until(now, ask.keep)?;
    let id = match credentials::named(connection, &owner, name)? {
        Some(saved) if !saved.same_fields(&ask.fields) => return Ok(Err(saved.fields)),
        Some(saved) => {
            credentials::renew(connection, &saved, &values, keep_until, now, seal)?;
            saved.id
        }
        None => credentials::insert(
            connection,
            &owner,
            &NewCredential {
                name: name.to_owned(),
                note: ask.reason.clone(),
                fields: ask.fields.clone(),
                values,
                keep_until,
            },
            now,
            seal,
        )?,
    };
    let credential = credentials::by_id(connection, &owner, &id)?.expect("the credential was just written");
    let others = credentials::waiting_for(connection, &owner, &[name])?;
    Ok(Ok(Answered {
        credential,
        keep: ask.keep,
        others: others.into_iter().collect(),
    }))
}

/// Whether `values` holds a value for each of `fields`
/// and nothing else.
fn fitting(
    fields: &[CredentialField],
    values: &BTreeMap<String, String>,
) -> Result<(), AnswerProblem> {
    for field in fields {
        let label = field.label.as_str().to_owned();
        match values.get(&field.name) {
            None => return Err(AnswerProblem::ValueMissing { label }),
            Some(_) => {}
        }
    }
    if let Some(other) = values
        .keys()
        .find(|name| !fields.iter().any(|field| &field.name == *name))
    {
        return Err(AnswerProblem::UnknownField { name: other.clone() });
    }
    Ok(())
}

/// Records the user's grant of `category` to `conversation`, through
/// [`ControlService::decide_permission`], and allows its undecided permission
/// requests for it.
fn grant(
    transaction: &rusqlite::Transaction<'_>,
    conversation: &ConversationId,
    category: &str,
    now: Timestamp,
) -> Result<Vec<StoredRequest>, StorageError> {
    transaction.execute(
        "INSERT OR IGNORE INTO permission_grants (conversation_id, category, granted_at)
         VALUES (?1, ?2, ?3)",
        params![conversation.as_str(), category, now.as_millisecond()],
    )?;
    let undecided: Vec<String> = transaction
        .prepare_cached(
            "SELECT id FROM permission_requests
             WHERE conversation_id = ?1 AND kind = 'permission' AND categories = ?2
               AND decision IS NULL",
        )?
        .query_map(params![conversation.as_str(), to_json(&[category])], |row| row.get(0))?
        .collect::<Result<_, _>>()?;
    decide(transaction, undecided, PermissionOutcome::Allowed, now)
}

/// Decides the requests `ids` with `outcome`, answering each as stored after.
fn decide(
    transaction: &rusqlite::Transaction<'_>,
    ids: Vec<String>,
    outcome: PermissionOutcome,
    now: Timestamp,
) -> Result<Vec<StoredRequest>, StorageError> {
    let decision = outcome_text(outcome);
    let mut requests = Vec::with_capacity(ids.len());
    for id in ids {
        transaction.execute(
            "UPDATE permission_requests SET decision = ?2, decided_at = ?3 WHERE id = ?1",
            params![id, decision, now.as_millisecond()],
        )?;
        let row = transaction.query_row(
            &format!("SELECT {REQUEST_COLUMNS} FROM permission_requests WHERE id = ?1"),
            [&id],
            |row| Ok(request_row(row)),
        )??;
        requests.push(row);
    }
    Ok(requests)
}

/// When the conversation was granted `category`, if it was.
fn granted_at(
    connection: &Connection,
    conversation: &ConversationId,
    category: &str,
) -> Result<Option<i64>, StorageError> {
    Ok(connection
        .query_row(
            "SELECT granted_at FROM permission_grants WHERE conversation_id = ?1 AND category = ?2",
            params![conversation.as_str(), category],
            |row| row.get(0),
        )
        .optional()?)
}

/// The category ids a `categories` column holds.
fn categories(stored: &str) -> Result<Vec<String>, StorageError> {
    let Categories(ids) = json(REQUESTS, "categories", stored)?;
    Ok(ids)
}

fn outcome_text(outcome: PermissionOutcome) -> &'static str {
    match outcome {
        PermissionOutcome::Allowed => "allowed",
        PermissionOutcome::Denied => "denied",
        PermissionOutcome::Reached => "reached",
    }
}

/// What the stored request asks, by its kind.
fn ask_row(row: &Row<'_>) -> Result<StoredAsk, StorageError> {
    let kind: String = row.get("kind")?;
    let name: Option<String> = row.get("credential_name")?;
    let shape = |reason: &str| StorageError::Corrupt {
        table: REQUESTS,
        column: "kind",
        reason: format!("a {kind} request {reason}"),
    };
    match kind.as_str() {
        "permission" => {
            let stored: Option<String> = row.get("categories")?;
            let stored = stored.ok_or_else(|| shape("without categories"))?;
            let mut categories = categories(&stored)?;
            match categories.len() {
                1 => Ok(StoredAsk::Permission(categories.remove(0))),
                _ => Err(shape("of other than one category")),
            }
        }
        "credential" => {
            let stored: Option<String> = row.get("ask")?;
            let stored = stored.ok_or_else(|| shape("without its ask"))?;
            Ok(StoredAsk::Credential {
                name: name.ok_or_else(|| shape("without a name"))?,
                ask: json(REQUESTS, "ask", &stored)?,
            })
        }
        other => Err(StorageError::Corrupt {
            table: REQUESTS,
            column: "kind",
            reason: format!("{other:?} is no kind of request"),
        }),
    }
}

fn request_row(row: &Row<'_>) -> Result<StoredRequest, StorageError> {
    let number: Option<i64> = row.get("agent_number")?;
    let description: Option<String> = row.get("agent_description")?;
    let subagent = match (number, description) {
        (Some(number), Some(description)) => Some(RequestingAgent {
            number: decode(REQUESTS, "agent_number", u64::try_from(number))?,
            description,
        }),
        (None, None) => None,
        _ => {
            return Err(StorageError::Corrupt {
                table: REQUESTS,
                column: "agent_number",
                reason: "a number without a description, or the reverse".into(),
            });
        }
    };
    let decision: Option<String> = row.get("decision")?;
    let outcome = match decision.as_deref() {
        None => None,
        Some("allowed") => Some(PermissionOutcome::Allowed),
        Some("denied") => Some(PermissionOutcome::Denied),
        Some("reached") => Some(PermissionOutcome::Reached),
        Some(other) => {
            return Err(StorageError::Corrupt {
                table: REQUESTS,
                column: "decision",
                reason: format!("{other:?} is no decision"),
            });
        }
    };
    let decision = match outcome {
        Some(outcome) => Some(Decision {
            outcome,
            at: instant(row, REQUESTS, "decided_at")?,
        }),
        None => None,
    };
    Ok(StoredRequest {
        id: decode(
            REQUESTS,
            "id",
            PermissionRequestId::try_from(row.get::<_, String>("id")?),
        )?,
        conversation: decode(
            REQUESTS,
            "conversation_id",
            ConversationId::try_from(row.get::<_, String>("conversation_id")?.as_str()),
        )?,
        ask: ask_row(row)?,
        command: row.get("command")?,
        agent: AskingAgent {
            node: decode(
                REQUESTS,
                "node_id",
                NodeId::try_from(row.get::<_, String>("node_id")?),
            )?,
            subagent,
        },
        created_at: instant(row, REQUESTS, "created_at")?,
        decision,
    })
}
