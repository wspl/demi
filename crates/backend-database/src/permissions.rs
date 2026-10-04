//! Conversation permissions (`storage.md` § Control records,
//! `permissions.md`): the requests refused commands raise, each undecided
//! until the user decides it and then kept until its message is in the
//! asking agent's checkpoint, and the grants, one per conversation and
//! category, each kept for the conversation's life. Each change reads and
//! writes in one transaction, so a check and a decision never interleave.

use demi_shared_types::{NodeId, PermissionOutcome, Timestamp};
use demi_web_api_protocol::ids::{ConversationId, UserId};
use demi_web_api_protocol::permissions::{PermissionRequestId, RequestingAgent};
use rusqlite::{Connection, OptionalExtension, Row, params};

use super::StorageError;
use super::columns::{decode, instant};
use super::control::ControlService;
use super::drafts::archived;

const REQUESTS: &str = "permission_requests";

const REQUEST_COLUMNS: &str =
    "id, conversation_id, category, command, node_id, agent_number, agent_description, created_at, decision, decided_at";

/// The agent that ran a refused command: its node, and, for a subagent, the
/// number and description the model knows it by.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AskingAgent {
    pub node: NodeId,
    /// None for the root.
    pub subagent: Option<RequestingAgent>,
}

/// A request as it is stored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredRequest {
    pub id: PermissionRequestId,
    pub conversation: ConversationId,
    pub category: String,
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
    /// The conversation has the grant: the call is dispatched.
    Granted,
    /// No grant: the request was recorded, in place of the undecided one of
    /// the same category from the same agent, if any.
    Raised(PermissionRequestId),
}

/// Why a decision changed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionRefusal {
    /// The conversation is archived.
    Archived,
    /// No undecided request of that id in the conversation.
    NotFound,
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
    /// conversation has the grant of `category`, and when it has not, the
    /// request recorded for `command`, which replaces the undecided request
    /// of the same category from the same agent.
    pub async fn check_permission(
        &self,
        conversation: ConversationId,
        category: String,
        command: String,
        agent: AskingAgent,
    ) -> Result<Checked, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            if granted_at(&transaction, &conversation, &category)?.is_some() {
                return Ok(Checked::Granted);
            }
            transaction.execute(
                "DELETE FROM permission_requests
                 WHERE conversation_id = ?1 AND category = ?2 AND node_id = ?3 AND decision IS NULL",
                params![conversation.as_str(), category, agent.node.as_str()],
            )?;
            let id = PermissionRequestId::try_from(uuid::Uuid::new_v4().to_string())
                .expect("a UUID is not empty");
            let (number, description) = match &agent.subagent {
                Some(subagent) => (
                    Some(i64::try_from(subagent.number).expect("an agent number fits")),
                    Some(subagent.description.as_str()),
                ),
                None => (None, None),
            };
            transaction.execute(
                "INSERT INTO permission_requests
                 (id, conversation_id, category, command, node_id, agent_number, agent_description, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                params![
                    id.as_str(),
                    conversation.as_str(),
                    category,
                    command,
                    agent.node.as_str(),
                    number,
                    description,
                    now.as_millisecond()
                ],
            )?;
            transaction.commit()?;
            Ok(Checked::Raised(id))
        })
        .await
    }

    /// The conversation's undecided requests, oldest first.
    pub async fn permission_requests(
        &self,
        conversation: ConversationId,
    ) -> Result<Vec<StoredRequest>, StorageError> {
        self.call(move |connection, _| {
            connection
                .prepare_cached(&format!(
                    "SELECT {REQUEST_COLUMNS} FROM permission_requests
                     WHERE conversation_id = ?1 AND decision IS NULL ORDER BY created_at, rowid"
                ))?
                .query_map([conversation.as_str()], |row| Ok(request_row(row)))?
                .map(|row| row?)
                .collect()
        })
        .await
    }

    /// Decides the undecided request `request` (`permissions.md`
    /// § Requests). An allow records the grant of its category and decides
    /// every undecided request of that category in the conversation; a deny
    /// decides this request alone. Answers the requests it decided, whose
    /// messages the backend delivers next.
    pub async fn decide_permission(
        &self,
        conversation: ConversationId,
        request: PermissionRequestId,
        outcome: PermissionOutcome,
    ) -> Result<Result<Vec<StoredRequest>, PermissionRefusal>, StorageError> {
        self.call(move |connection, now| {
            let transaction = connection.transaction()?;
            if archived(&transaction, &conversation)? {
                return Ok(Err(PermissionRefusal::Archived));
            }
            let category: Option<String> = transaction
                .query_row(
                    "SELECT category FROM permission_requests
                     WHERE id = ?1 AND conversation_id = ?2 AND decision IS NULL",
                    params![request.as_str(), conversation.as_str()],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(category) = category else {
                return Ok(Err(PermissionRefusal::NotFound));
            };
            let decided: Vec<String> = match outcome {
                PermissionOutcome::Allowed => {
                    transaction.execute(
                        "INSERT OR IGNORE INTO permission_grants (conversation_id, category, granted_at)
                         VALUES (?1, ?2, ?3)",
                        params![conversation.as_str(), category, now.as_millisecond()],
                    )?;
                    transaction
                        .prepare_cached(
                            "SELECT id FROM permission_requests
                             WHERE conversation_id = ?1 AND category = ?2 AND decision IS NULL",
                        )?
                        .query_map(params![conversation.as_str(), category], |row| row.get(0))?
                        .collect::<Result<_, _>>()?
                }
                PermissionOutcome::Denied => vec![request.as_str().to_owned()],
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

fn outcome_text(outcome: PermissionOutcome) -> &'static str {
    match outcome {
        PermissionOutcome::Allowed => "allowed",
        PermissionOutcome::Denied => "denied",
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
        category: row.get("category")?,
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
