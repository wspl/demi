//! The usage ledger of the control store (`usage-and-quota.md` § Usage
//! ledger): one row per answered request, kept as written, and the totals
//! computed when they are read.

use demi_core::TokenUsage;
use demi_web_api::ids::{ConversationId, ProviderId, UserId};
use demi_web_api::text::EmailAddress;
use demi_web_api::usage::{UsageGroup, UserUsage};
use rusqlite::{Connection, Row, params};

use super::StorageError;
use super::columns::decode;
use super::control::ControlService;

/// One answered request: who made it, in which conversation, with which
/// entry and model, and the tokens its response reported.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(
    not(test),
    expect(dead_code, reason = "the metered runtime writes it once the agent runs one (4D)")
)]
pub(crate) struct UsageRow {
    pub(crate) user: UserId,
    pub(crate) conversation: ConversationId,
    pub(crate) provider: ProviderId,
    pub(crate) model: String,
    pub(crate) usage: TokenUsage,
}

/// One user's totals: a group per entry and model, in the order each pair
/// was first used.
const TOTALS: &str = "SELECT provider_id, model_id, COUNT(*) AS requests,
       SUM(input_tokens) AS input_tokens, SUM(output_tokens) AS output_tokens,
       SUM(cache_read_tokens) AS cache_read_tokens, SUM(cache_write_tokens) AS cache_write_tokens
     FROM usage_ledger WHERE user_id = ?1
     GROUP BY provider_id, model_id
     ORDER BY MIN(created_at), MIN(rowid)";

impl ControlService {
    /// Writes the row, timed now.
    #[cfg_attr(
        not(test),
        expect(dead_code, reason = "the metered runtime writes rows once the agent runs one (4D)")
    )]
    pub(crate) async fn append_usage(&self, row: UsageRow) -> Result<(), StorageError> {
        let id = uuid::Uuid::new_v4().to_string();
        self.call(move |connection, now| {
            let tokens = [
                row.usage.input_tokens,
                row.usage.output_tokens,
                row.usage.cache_read_tokens,
                row.usage.cache_write_tokens,
            ]
            // A count beyond i64 is no count a vendor reports; the column
            // keeps it as the largest it holds rather than failing the row.
            .map(|count| i64::try_from(count).unwrap_or(i64::MAX));
            connection.execute(
                "INSERT INTO usage_ledger (id, user_id, conversation_id, provider_id, model_id,
                   input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                params![
                    id,
                    row.user.as_str(),
                    row.conversation.as_str(),
                    row.provider.as_str(),
                    row.model,
                    tokens[0],
                    tokens[1],
                    tokens[2],
                    tokens[3],
                    now.as_millisecond()
                ],
            )?;
            Ok(())
        })
        .await
    }

    /// The user's totals.
    pub(crate) async fn usage_totals(&self, user: UserId) -> Result<Vec<UsageGroup>, StorageError> {
        self.call(move |connection, _| totals(connection, &user)).await
    }

    /// Every account's totals, the accounts in the order they were created,
    /// read in one transaction.
    pub(crate) async fn instance_usage(&self) -> Result<Vec<UserUsage>, StorageError> {
        self.call(|connection, _| {
            let transaction = connection.transaction()?;
            let users = {
                let mut statement =
                    transaction.prepare_cached("SELECT id, email FROM users ORDER BY created_at, rowid")?;
                let mut rows = statement.query([])?;
                let mut users = Vec::new();
                while let Some(row) = rows.next()? {
                    let id = decode("users", "id", UserId::try_from(row.get::<_, String>("id")?))?;
                    let email = decode("users", "email", EmailAddress::try_from(row.get::<_, String>("email")?))?;
                    users.push((id, email));
                }
                users
            };
            let mut usage = Vec::with_capacity(users.len());
            for (user_id, email) in users {
                let totals = totals(&transaction, &user_id)?;
                usage.push(UserUsage { user_id, email, totals });
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
        decode("usage_ledger", column, u64::try_from(row.get::<_, i64>(column)?))
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
