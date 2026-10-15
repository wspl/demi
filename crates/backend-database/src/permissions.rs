//! A conversation's requests (`storage.md` § Control records,
//! `permissions.md` § Requests, `credentials.md`): the permission requests
//! refused commands raise, the credential requests of agents that wait for
//! the user to save a credential, and the allowance requests of reads of a
//! credential the conversation may not use yet; each undecided until the
//! user decides it, and a decided one kept until its message is in the
//! asking agent's checkpoint. And what the decisions record: the grants,
//! one per conversation and category, and the allowances, one per
//! conversation and credential, each kept for the conversation's life. A
//! permission request names the categories its call lacked, sorted, so that
//! the same categories compare equal, and the host it asks to reach, if any
//! (`permissions.md` § Hosts): an allow puts a device on the list of hosts
//! the conversation may reach, or lets it pair a computer's code once. Each
//! change reads and writes in one transaction, so a check and a decision
//! never interleave.

use std::collections::BTreeMap;

use demi_shared_types::{NodeId, PermissionOutcome, Timestamp};
use demi_web_api_protocol::credentials::{CredentialField, Keep, VALUE_MAX_BYTES};
use demi_web_api_protocol::ids::{ConversationId, DeviceId, UserCredentialId, UserId};
use demi_web_api_protocol::permissions::{PermissionRequestId, RequestingAgent};
use garde::Validate;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde::{Deserialize, Serialize};

use super::StorageError;
use super::columns::{decode, instant, json, to_json};
use super::control::ControlService;
use super::conversation_index::{add_reachable_host, may_reach};
use super::credentials::{
    self, CredentialSeal, NewCredential, StoredCredential, values_of,
};
use super::drafts::archived;

const REQUESTS: &str = "permission_requests";

const REQUEST_COLUMNS: &str = "id, conversation_id, kind, categories, credential_name, credential_id, ask, command, node_id, agent_number, agent_description, created_at, decision, decided_at, host_device_id, pairing_code";

/// A request's categories as its column holds them: their ids, sorted; none
/// when the request asks for a host alone.
#[derive(Debug, Serialize, Deserialize, Validate)]
#[garde(transparent)]
struct Categories(#[garde(inner(length(min = 1)))] Vec<String>);

/// The host a call brings into its conversation (`permissions.md` § Hosts).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AskedHost {
    /// A device of the user's, the Cloud's included.
    Device(DeviceId),
    /// The computer `demi host pair` pairs, by the code its installer shows.
    Pairing(String),
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
    /// The keep the card offers first.
    #[garde(skip)]
    pub keep: Keep,
}

/// What a request asks of the user.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoredAsk {
    /// The ids of the categories the call lacked, sorted, and the host it
    /// asks to reach; at least one of the two.
    Permission {
        categories: Vec<String>,
        host: Option<AskedHost>,
    },
    /// A credential of this name to save.
    Credential { name: String, ask: CredentialAsk },
    /// The use of the credential `credential`, none once it was removed.
    Allowance {
        name: String,
        credential: Option<UserCredentialId>,
    },
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
    /// A grant or a host is missing: the request was recorded for the
    /// categories `lacking`, sorted, and the host `host`, in place of the
    /// undecided one of the same categories and host from the same agent,
    /// if any.
    Raised {
        id: PermissionRequestId,
        lacking: Vec<String>,
        host: Option<AskedHost>,
    },
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
    /// The credential of this name, which the request asks to use, was
    /// removed.
    Removed(String),
}

/// What answering a credential request saved, with the other conversations
/// whose credential requests of its name wait, whose renewal follows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answered {
    pub credential: StoredCredential,
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

/// What a read of a credential's field found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CredentialRead {
    /// The field's value; the credential's last use is now.
    Value(String),
    /// The user has no credential of that name.
    NoCredential,
    /// The credential has no field of that name, but these.
    NoField(Vec<CredentialField>),
    /// The conversation may not use the credential of this name, as it is
    /// saved, yet: a request asks the user, in place of the undecided one of
    /// the same credential from the same agent.
    Raised(String),
}

/// A decided request whose message the backend still holds, with the owner
/// of its conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Undelivered {
    pub owner: UserId,
    pub request: StoredRequest,
}

impl ControlService {
    /// The check of one call (`permissions.md` § The check): whether the
    /// conversation has the grant of each of `categories` and may reach
    /// `host`, and when it lacks any, the request recorded for `command`
    /// with what it lacks, which replaces the undecided request of the same
    /// categories and host from the same agent. A pairing code the user let
    /// the conversation pair is used up by the call it passes.
    pub async fn check_permission(
        &self,
        conversation: ConversationId,
        categories: Vec<String>,
        host: Option<AskedHost>,
        command: String,
        agent: AskingAgent,
    ) -> Result<Checked, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let mut lacking = Vec::new();
            for category in categories {
                if granted_at(&transaction, &conversation, &category)?.is_none() {
                    lacking.push(category);
                }
            }
            let reached = match &host {
                Some(host) => reaches(&transaction, &conversation, host)?,
                None => true,
            };
            if lacking.is_empty() && reached {
                if let Some(AskedHost::Pairing(code)) = &host {
                    transaction.execute(
                        "DELETE FROM pairing_passes WHERE conversation_id = ?1 AND code = ?2",
                        params![conversation.as_str(), code],
                    )?;
                    transaction.commit()?;
                }
                return Ok(Checked::Granted);
            }
            let host = host.filter(|_| !reached);
            lacking.sort();
            lacking.dedup();
            let stored = to_json(&lacking);
            let (device, code) = host_columns(host.as_ref());
            transaction.execute(
                "DELETE FROM permission_requests
                 WHERE conversation_id = ?1 AND kind = 'permission' AND categories = ?2 AND node_id = ?3
                   AND host_device_id IS ?4 AND pairing_code IS ?5 AND decision IS NULL",
                params![conversation.as_str(), stored, agent.node.as_str(), device, code],
            )?;
            let id = new_request_id();
            insert_request(
                &transaction,
                &id,
                &conversation,
                &Columns {
                    kind: "permission",
                    categories: Some(stored),
                    host_device_id: device.map(str::to_owned),
                    pairing_code: code.map(str::to_owned),
                    ..Columns::default()
                },
                &command,
                &agent,
                now,
            )?;
            transaction.commit()?;
            Ok(Checked::Raised { id, lacking, host })
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

    /// Decides the undecided request `request` (`permissions.md`
    /// § Requests). An allow of a permission request records the grant of
    /// each of its categories, puts its device on the conversation's list
    /// or lets the conversation pair its code once, and decides it with
    /// every undecided permission request of the conversation whose
    /// categories are now all granted and whose host it may reach; an
    /// allow of an allowance request records the allowance and decides
    /// every undecided request of the conversation to use that credential;
    /// a deny decides the request alone, and forgets a credential request,
    /// whose agent waits for the answer rather than a message. A credential
    /// request is allowed only with its values ([`Self::answer_credential`]).
    /// Answers the requests it decided, whose messages the backend delivers
    /// next, and the credential request it denied.
    pub async fn decide_permission(
        &self,
        conversation: ConversationId,
        request: PermissionRequestId,
        outcome: PermissionOutcome,
    ) -> Result<Result<Vec<StoredRequest>, PermissionRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            let decided = match undecided(&transaction, &conversation, &request)? {
                Err(refusal) => return Ok(Err(refusal)),
                Ok(stored) => match (&stored.ask, outcome) {
                    (StoredAsk::Credential { .. }, PermissionOutcome::Allowed) => {
                        return Ok(Err(PermissionRefusal::InvalidAnswer(
                            AnswerProblem::ValuesMissing,
                        )));
                    }
                    (StoredAsk::Credential { .. }, PermissionOutcome::Denied) => {
                        transaction.execute(
                            "DELETE FROM permission_requests WHERE id = ?1",
                            [request.as_str()],
                        )?;
                        transaction.commit()?;
                        return Ok(Ok(vec![stored]));
                    }
                    (StoredAsk::Permission { categories, host }, PermissionOutcome::Allowed) => {
                        for category in categories {
                            transaction.execute(
                                "INSERT OR IGNORE INTO permission_grants (conversation_id, category, granted_at)
                                 VALUES (?1, ?2, ?3)",
                                params![conversation.as_str(), category, now.as_millisecond()],
                            )?;
                        }
                        match host {
                            Some(AskedHost::Device(device)) => {
                                add_reachable_host(&transaction, &conversation, device, now)?;
                            }
                            Some(AskedHost::Pairing(code)) => {
                                transaction.execute(
                                    "INSERT OR IGNORE INTO pairing_passes (conversation_id, code, allowed_at)
                                     VALUES (?1, ?2, ?3)",
                                    params![conversation.as_str(), code, now.as_millisecond()],
                                )?;
                            }
                            None => {}
                        }
                        let undecided: Vec<String> = transaction
                            .prepare_cached(
                                "SELECT id FROM permission_requests
                                 WHERE conversation_id = ?1 AND kind = 'permission' AND decision IS NULL
                                   AND id <> ?2",
                            )?
                            .query_map(params![conversation.as_str(), request.as_str()], |row| row.get(0))?
                            .collect::<Result<_, _>>()?;
                        // The request decided is complete even when its
                        // device went meanwhile and joined no list.
                        let mut complete = vec![request.as_str().to_owned()];
                        for id in undecided {
                            let other = transaction.query_row(
                                &format!("SELECT {REQUEST_COLUMNS} FROM permission_requests WHERE id = ?1"),
                                [&id],
                                |row| Ok(ask_row(row)),
                            )??;
                            let StoredAsk::Permission { categories, host } = other else {
                                continue;
                            };
                            let mut granted = true;
                            for category in &categories {
                                granted &= granted_at(&transaction, &conversation, category)?.is_some();
                            }
                            if let Some(host) = &host {
                                granted &= reaches(&transaction, &conversation, host)?;
                            }
                            if granted {
                                complete.push(id);
                            }
                        }
                        complete
                    }
                    (StoredAsk::Allowance { name, credential: None }, PermissionOutcome::Allowed) => {
                        return Ok(Err(PermissionRefusal::Removed(name.clone())));
                    }
                    (StoredAsk::Allowance { credential: Some(credential), .. }, PermissionOutcome::Allowed) => {
                        allow(&transaction, &conversation, credential, now)?;
                        transaction
                            .prepare_cached(
                                "SELECT id FROM permission_requests
                                 WHERE conversation_id = ?1 AND credential_id = ?2 AND decision IS NULL",
                            )?
                            .query_map(params![conversation.as_str(), credential.as_str()], |row| {
                                row.get(0)
                            })?
                            .collect::<Result<_, _>>()?
                    }
                    (_, PermissionOutcome::Denied) => vec![request.as_str().to_owned()],
                },
            };
            let decision = outcome_text(outcome);
            let mut requests = Vec::with_capacity(decided.len());
            for id in decided {
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
            transaction.commit()?;
            Ok(Ok(requests))
        })
        .await
    }

    /// Records a credential request of the agent `agent` (`credentials.md`
    /// § Commands) as `id`, unless the user has a credential of the name
    /// with other fields.
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

    /// Answers the credential request `request` with `values`, kept as
    /// `keep` says (`credentials.md` § Requests and routes), in one
    /// transaction: it saves a new credential of the request's name, or new
    /// values of the saved one with the same fields; allows the
    /// conversation to use it; and forgets the request, whose agent waits
    /// for the answer.
    pub async fn answer_credential(
        &self,
        conversation: ConversationId,
        request: PermissionRequestId,
        values: BTreeMap<String, String>,
        keep: Keep,
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
            let owner = owner_of(&transaction, &conversation)?;
            let keep_until = credentials::keep_until(now, keep)?;
            let id = match credentials::named(&transaction, &owner, name)? {
                Some(saved) if !saved.same_fields(&ask.fields) => {
                    return Ok(Err(PermissionRefusal::FieldsDiffer(format!(
                        "\u{201c}{name}\u{201d} already has other fields; answer with the same fields or choose another name."
                    ))));
                }
                Some(saved) => {
                    credentials::renew(&transaction, &saved, &values, keep_until, now, &*seal)?;
                    saved.id
                }
                None => credentials::insert(
                    &transaction,
                    &owner,
                    &NewCredential {
                        name: name.clone(),
                        note: ask.reason.clone(),
                        fields: ask.fields.clone(),
                        values,
                        keep_until,
                    },
                    now,
                    &*seal,
                )?,
            };
            allow(&transaction, &conversation, &id, now)?;
            transaction.execute("DELETE FROM permission_requests WHERE id = ?1", [request.as_str()])?;
            let credential =
                credentials::by_id(&transaction, &owner, &id)?.expect("the credential was just written");
            let others = credentials::waiting_for(&transaction, &owner, &[name])?;
            transaction.commit()?;
            Ok(Ok(Answered {
                credential,
                others: others.into_iter().collect(),
            }))
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

    /// The field `field` of the user's credential `name`, for the agent
    /// `agent` of the conversation (`credentials.md` § Using a credential):
    /// its value when the conversation may use the credential, which is
    /// then its last use; otherwise the allowance request recorded for
    /// `command`, in place of the undecided one of the same credential from
    /// the same agent.
    pub async fn read_credential(
        &self,
        conversation: ConversationId,
        name: String,
        field: String,
        command: String,
        agent: AskingAgent,
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
            let allowed: bool = transaction.query_row(
                "SELECT EXISTS (SELECT 1 FROM credential_allowances
                                WHERE conversation_id = ?1 AND credential_id = ?2)",
                params![conversation.as_str(), stored.id.as_str()],
                |row| row.get(0),
            )?;
            if !allowed {
                transaction.execute(
                    "DELETE FROM permission_requests
                     WHERE conversation_id = ?1 AND kind = 'allowance' AND credential_id = ?2
                       AND node_id = ?3 AND decision IS NULL",
                    params![conversation.as_str(), stored.id.as_str(), agent.node.as_str()],
                )?;
                let id = new_request_id();
                insert_request(
                    &transaction,
                    &id,
                    &conversation,
                    &Columns {
                        kind: "allowance",
                        credential_name: Some(stored.name.clone()),
                        credential_id: Some(stored.id.as_str().to_owned()),
                        ..Columns::default()
                    },
                    &command,
                    &agent,
                    now,
                )?;
                transaction.commit()?;
                return Ok(CredentialRead::Raised(stored.name));
            }
            let mut values = values_of(&transaction, &stored, &*seal)?;
            let value = values.remove(&field).ok_or_else(|| StorageError::Corrupt {
                table: credentials::TABLE,
                column: "sealed",
                reason: format!("the credential \u{201c}{name}\u{201d} holds no value of its field \u{201c}{field}\u{201d}"),
            })?;
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
    credential_id: Option<String>,
    ask: Option<String>,
    host_device_id: Option<String>,
    pairing_code: Option<String>,
}

/// The `host_device_id` and `pairing_code` columns of `host`.
fn host_columns(host: Option<&AskedHost>) -> (Option<&str>, Option<&str>) {
    match host {
        None => (None, None),
        Some(AskedHost::Device(device)) => (Some(device.as_str()), None),
        Some(AskedHost::Pairing(code)) => (None, Some(code.as_str())),
    }
}

/// Whether the conversation may reach `host`: a device it may reach, or a
/// pairing code the user let it pair.
fn reaches(connection: &Connection, conversation: &ConversationId, host: &AskedHost) -> Result<bool, StorageError> {
    match host {
        AskedHost::Device(device) => may_reach(connection, conversation, device),
        AskedHost::Pairing(code) => Ok(connection.query_row(
            "SELECT EXISTS (SELECT 1 FROM pairing_passes WHERE conversation_id = ?1 AND code = ?2)",
            params![conversation.as_str(), code],
            |row| row.get(0),
        )?),
    }
}

fn new_request_id() -> PermissionRequestId {
    PermissionRequestId::try_from(uuid::Uuid::new_v4().to_string()).expect("a UUID is not empty")
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
         (id, conversation_id, kind, categories, credential_name, credential_id, ask, command,
          node_id, agent_number, agent_description, created_at, host_device_id, pairing_code)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
        params![
            id.as_str(),
            conversation.as_str(),
            columns.kind,
            columns.categories,
            columns.credential_name,
            columns.credential_id,
            columns.ask,
            command,
            agent.node.as_str(),
            number,
            description,
            now.as_millisecond(),
            columns.host_device_id,
            columns.pairing_code
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

/// Allows the conversation to use the credential.
fn allow(
    connection: &Connection,
    conversation: &ConversationId,
    credential: &UserCredentialId,
    now: Timestamp,
) -> Result<(), StorageError> {
    connection.execute(
        "INSERT OR IGNORE INTO credential_allowances (conversation_id, credential_id, allowed_at)
         VALUES (?1, ?2, ?3)",
        params![conversation.as_str(), credential.as_str(), now.as_millisecond()],
    )?;
    Ok(())
}

/// Why an answer does not fit its request: its English is the refusal's
/// message, and the field and the limit it names are the values the page
/// words it with (`localization.md` § Text from the backend).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AnswerProblem {
    /// An allow of a credential request came without its values.
    #[error("Enter a value for each field to save the credential.")]
    ValuesMissing,
    /// The field labelled `label` has no value.
    #[error("Enter a value for \u{201c}{label}\u{201d}.")]
    ValueMissing { label: String },
    /// The value of the field labelled `label` is over `limit_bytes`.
    #[error("The value of \u{201c}{label}\u{201d} can be at most 64 KiB.")]
    ValueTooLarge { label: String, limit_bytes: usize },
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

/// Whether `values` holds a value of at most 64 KiB for each of `fields`
/// and nothing else (`web-api.md` § Conversation permissions).
fn fitting(
    fields: &[CredentialField],
    values: &BTreeMap<String, String>,
) -> Result<(), AnswerProblem> {
    for field in fields {
        let label = field.label.as_str().to_owned();
        match values.get(&field.name) {
            None => return Err(AnswerProblem::ValueMissing { label }),
            Some(value) if value.len() > VALUE_MAX_BYTES => {
                return Err(AnswerProblem::ValueTooLarge {
                    label,
                    limit_bytes: VALUE_MAX_BYTES,
                });
            }
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
            let categories = categories(&stored)?;
            let device: Option<String> = row.get("host_device_id")?;
            let code: Option<String> = row.get("pairing_code")?;
            let host = match (device, code) {
                (Some(device), None) => Some(AskedHost::Device(decode(
                    REQUESTS,
                    "host_device_id",
                    DeviceId::try_from(device),
                )?)),
                (None, Some(code)) => Some(AskedHost::Pairing(code)),
                (None, None) => None,
                (Some(_), Some(_)) => return Err(shape("with a device and a pairing code")),
            };
            if categories.is_empty() && host.is_none() {
                return Err(shape("of neither a category nor a host"));
            }
            Ok(StoredAsk::Permission { categories, host })
        }
        "credential" => {
            let stored: Option<String> = row.get("ask")?;
            let stored = stored.ok_or_else(|| shape("without its ask"))?;
            Ok(StoredAsk::Credential {
                name: name.ok_or_else(|| shape("without a name"))?,
                ask: json(REQUESTS, "ask", &stored)?,
            })
        }
        "allowance" => {
            let credential: Option<String> = row.get("credential_id")?;
            Ok(StoredAsk::Allowance {
                name: name.ok_or_else(|| shape("without a name"))?,
                credential: credential
                    .map(|id| decode(REQUESTS, "credential_id", UserCredentialId::try_from(id)))
                    .transpose()?,
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
