//! The usage ledger of the control store (`usage-and-quota.md` § Usage
//! ledger): one row per vendor call, kept as written, and the totals
//! computed when they are read.

use demi_shared_types::{QuotaKind, QuotaScope, QuotaWindow, Timestamp, TokenUsage};
use demi_web_api_protocol::ids::{ConversationId, CredentialId, ProviderId, UserId};
use demi_web_api_protocol::text::EmailAddress;
use demi_web_api_protocol::usage::{UsageGroup, UserUsage};
use rusqlite::{Connection, Row, params};
use serde::{Deserialize, Serialize};

use super::StorageError;
use super::columns::{decode, json, to_json};
use super::control::ControlService;

/// One call to a vendor: whose it was, what it was for, with which entry,
/// account and model, the vendor's counts for it alone, how it ended and
/// the account's quota observed during it.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageRow {
    pub user: UserId,
    pub conversation: ConversationId,
    /// The session's agent number, 0 for the root; none outside a session.
    pub agent: Option<u64>,
    /// None outside a turn.
    pub turn: Option<String>,
    /// `turn`, `compaction` or `title`.
    pub purpose: &'static str,
    pub provider: ProviderId,
    /// The subscription account; none for an API key.
    pub account: Option<CredentialId>,
    pub model: String,
    /// Demi's id of the request, under which the server log holds its
    /// course.
    pub request: String,
    pub vendor_id: Option<String>,
    /// The vendor's counts for the call; none when it reported none, and
    /// then every count, the hour-long cache write included, is null.
    pub usage: Option<TokenUsage>,
    pub cache_write_1h_tokens: u64,
    /// `answered`, `cancelled` or the failure code of a refused call.
    pub outcome: String,
    /// The windows the call's own response reported; none when it
    /// reported none.
    pub quota: Option<Vec<ObservedWindow>>,
    pub started_at: Timestamp,
    pub ended_at: Timestamp,
}

/// A window of the account's quota as a call's response reported it, as
/// the row's `quota` keeps it: named as the snapshot names a window, by its
/// kind, period and scope.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ObservedWindow {
    #[garde(skip)]
    pub kind: QuotaKind,
    #[serde(deserialize_with = "Option::deserialize")]
    #[garde(skip)]
    pub period_seconds: Option<u64>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[garde(skip)]
    pub scope: Option<QuotaScope>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[garde(range(min = 0.0, max = 100.0))]
    pub used_percent: Option<f64>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[garde(skip)]
    pub resets_at: Option<Timestamp>,
}

impl From<&QuotaWindow> for ObservedWindow {
    fn from(window: &QuotaWindow) -> Self {
        Self {
            kind: window.kind,
            period_seconds: window.period_seconds,
            scope: window.scope.clone(),
            used_percent: window.used_percent,
            resets_at: window.resets_at,
        }
    }
}

/// Adds `windows` to `kept`, each replacing a window of the same name, in
/// the order they were reported.
pub fn merge_windows(kept: &mut Vec<ObservedWindow>, windows: &[QuotaWindow]) {
    for window in windows {
        let window = ObservedWindow::from(window);
        let named = |kept: &ObservedWindow| {
            (kept.kind, kept.period_seconds, &kept.scope) == (window.kind, window.period_seconds, &window.scope)
        };
        match kept.iter_mut().find(|kept| named(kept)) {
            Some(kept) => *kept = window,
            None => kept.push(window),
        }
    }
}

/// One user's totals: a group per entry and model, in the order each pair
/// was first used.
/// A count the vendor did not report adds nothing.
const TOTALS: &str = "SELECT provider_id, model_id, COUNT(*) AS requests,
       COALESCE(SUM(input_tokens), 0) AS input_tokens, COALESCE(SUM(output_tokens), 0) AS output_tokens,
       COALESCE(SUM(cache_read_tokens), 0) AS cache_read_tokens,
       COALESCE(SUM(cache_write_tokens), 0) AS cache_write_tokens
     FROM usage_ledger WHERE user_id = ?1
     GROUP BY provider_id, model_id
     ORDER BY MIN(ended_at), MIN(rowid)";

/// A count as the column keeps it: a count beyond i64 is no count a vendor
/// reports, and the column keeps it as the largest it holds rather than
/// failing the row.
fn column_count(count: u64) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}

impl ControlService {
    /// Writes the row and answers its id.
    pub async fn append_usage(&self, row: UsageRow) -> Result<String, StorageError> {
        let id = uuid::Uuid::new_v4().to_string();
        self.call(move |connection, _| {
            let quota = row.quota.as_ref().map(to_json);
            let counts = row.usage.map(|usage| {
                [
                    usage.input_tokens,
                    usage.output_tokens,
                    usage.cache_read_tokens,
                    usage.cache_write_tokens,
                    row.cache_write_1h_tokens,
                ]
                .map(column_count)
            });
            let count = |index: usize| counts.map(|counts| counts[index]);
            connection.execute(
                "INSERT INTO usage_ledger (id, user_id, conversation_id, agent, turn_id, purpose,
                   provider_id, account_id, model_id, request_id, vendor_id,
                   input_tokens, output_tokens, cache_read_tokens, cache_write_tokens,
                   cache_write_1h_tokens, outcome, quota, started_at, ended_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17,
                   ?18, ?19, ?20)",
                params![
                    id,
                    row.user.as_str(),
                    row.conversation.as_str(),
                    row.agent.map(column_count),
                    row.turn,
                    row.purpose,
                    row.provider.as_str(),
                    row.account.as_ref().map(CredentialId::as_str),
                    row.model,
                    row.request,
                    row.vendor_id,
                    count(0),
                    count(1),
                    count(2),
                    count(3),
                    count(4),
                    row.outcome,
                    quota,
                    row.started_at.as_millisecond(),
                    row.ended_at.as_millisecond(),
                ],
            )?;
            Ok(id)
        })
        .await
    }

    /// Adds `windows`, which the vendor reported after the call of the row
    /// `id` ended and before the next one started, to the row's quota.
    pub async fn add_usage_quota(&self, id: String, windows: Vec<QuotaWindow>) -> Result<(), StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let kept: Option<String> =
                transaction.query_row("SELECT quota FROM usage_ledger WHERE id = ?1", [&id], |row| row.get(0))?;
            let mut quota: Vec<ObservedWindow> = match kept {
                Some(text) => json("usage_ledger", "quota", &text)?,
                None => Vec::new(),
            };
            merge_windows(&mut quota, &windows);
            transaction.execute(
                "UPDATE usage_ledger SET quota = ?2 WHERE id = ?1",
                params![id, to_json(&quota)],
            )?;
            transaction.commit()?;
            Ok(())
        })
        .await
    }

    /// The user's totals.
    pub async fn usage_totals(&self, user: UserId) -> Result<Vec<UsageGroup>, StorageError> {
        self.call(move |connection, _| totals(connection, &user))
            .await
    }

    /// Every account's totals, the accounts in the order they were created,
    /// read in one transaction.
    pub async fn instance_usage(&self) -> Result<Vec<UserUsage>, StorageError> {
        self.call(|connection, _| {
            let transaction = connection.transaction()?;
            let users = {
                let mut statement = transaction
                    .prepare_cached("SELECT id, email FROM users ORDER BY created_at, rowid")?;
                let mut rows = statement.query([])?;
                let mut users = Vec::new();
                while let Some(row) = rows.next()? {
                    let id = decode("users", "id", UserId::try_from(row.get::<_, String>("id")?))?;
                    let email = decode(
                        "users",
                        "email",
                        EmailAddress::try_from(row.get::<_, String>("email")?),
                    )?;
                    users.push((id, email));
                }
                users
            };
            let mut usage = Vec::with_capacity(users.len());
            for (user_id, email) in users {
                let totals = totals(&transaction, &user_id)?;
                usage.push(UserUsage {
                    user_id,
                    email,
                    totals,
                });
            }
            transaction.commit()?;
            Ok(usage)
        })
        .await
    }
}

fn totals(connection: &Connection, user: &UserId) -> Result<Vec<UsageGroup>, StorageError> {
    let mut statement = connection.prepare_cached(TOTALS)?;
    let mut rows = statement.query([user.as_str()])?;
    let mut groups = Vec::new();
    while let Some(row) = rows.next()? {
        groups.push(group(row)?);
    }
    Ok(groups)
}

fn group(row: &Row<'_>) -> Result<UsageGroup, StorageError> {
    let count = |column: &'static str| -> Result<u64, StorageError> {
        decode(
            "usage_ledger",
            column,
            u64::try_from(row.get::<_, i64>(column)?),
        )
    };
    Ok(UsageGroup {
        provider_id: row.get("provider_id")?,
        model_id: row.get("model_id")?,
        requests: count("requests")?,
        input_tokens: count("input_tokens")?,
        output_tokens: count("output_tokens")?,
        cache_read_tokens: count("cache_read_tokens")?,
        cache_write_tokens: count("cache_write_tokens")?,
    })
}
