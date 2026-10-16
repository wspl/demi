//! The sidebar's explicit order (`storage.md` § Control records): a move of
//! one row before another, or to the end, within its partition, which the
//! move writes whole in one transaction. Activity never reorders a row.

use demi_web_api_protocol::ids::{ConversationId, UserId, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, params};

use super::StorageError;
use super::control::ControlService;

/// What a move of a row in the sidebar's order did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reorder {
    Moved,
    /// The row moved or the one it goes before is not the user's, as one
    /// another page deleted meanwhile; nothing is written.
    Missing,
    /// Both are the user's, but not in one partition; nothing is written.
    OutOfPartition,
}

impl ControlService {
    /// Moves the user's conversation `moved` before `before`, or to the end,
    /// within its partition (`storage.md` § Control records): the
    /// conversations that are not archived, pinned as it is, and in the same
    /// workspace or in none. Nothing is written when either is not the
    /// user's, or not in that partition, an archived one included.
    pub async fn reorder_conversations(
        &self,
        user: UserId,
        moved: ConversationId,
        before: Option<ConversationId>,
    ) -> Result<Reorder, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let before = before.as_ref().map(ConversationId::as_str);
            if !owns(&transaction, "conversations", &user, moved.as_str(), before)? {
                return Ok(Reorder::Missing);
            }
            let partition = transaction
                .query_row(
                    "SELECT archived, pinned, target_workspace_id FROM conversations WHERE id = ?1 AND user_id = ?2",
                    params![moved.as_str(), user.as_str()],
                    |row| Ok((row.get::<_, bool>(0)?, row.get::<_, bool>(1)?, row.get::<_, Option<String>>(2)?)),
                )
                .optional()?;
            let Some((false, pinned, workspace)) = partition else {
                return Ok(Reorder::OutOfPartition);
            };
            let peers = {
                let mut statement = transaction.prepare_cached(
                    "SELECT id FROM conversations
                     WHERE user_id = ?1 AND archived = 0 AND pinned = ?2 AND target_workspace_id IS ?3
                     ORDER BY sort_order, id",
                )?;
                let rows = statement.query_map(params![user.as_str(), pinned, workspace], |row| row.get::<_, String>(0))?;
                rows.collect::<Result<Vec<String>, _>>()?
            };
            if !write_order(&transaction, "conversations", peers, moved.as_str(), before)? {
                return Ok(Reorder::OutOfPartition);
            }
            transaction.commit()?;
            Ok(Reorder::Moved)
        })
        .await
    }

    /// Moves the user's workspace `moved` before `before`, or to the end;
    /// `Missing` when either is not one of the user's workspaces.
    pub async fn reorder_workspaces(
        &self,
        user: UserId,
        moved: WorkspaceId,
        before: Option<WorkspaceId>,
    ) -> Result<Reorder, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let peers = {
                let mut statement = transaction.prepare_cached(
                    "SELECT id FROM workspaces WHERE user_id = ?1 ORDER BY sort_order, id",
                )?;
                let rows = statement.query_map([user.as_str()], |row| row.get::<_, String>(0))?;
                rows.collect::<Result<Vec<String>, _>>()?
            };
            let before = before.as_ref().map(WorkspaceId::as_str);
            // The user's workspaces are one partition: a row not in it is not the user's.
            if !write_order(&transaction, "workspaces", peers, moved.as_str(), before)? {
                return Ok(Reorder::Missing);
            }
            transaction.commit()?;
            Ok(Reorder::Moved)
        })
        .await
    }
}

/// Whether `moved`, and `before` when given, are rows of `user` in `table`.
fn owns(
    connection: &Connection,
    table: &'static str,
    user: &UserId,
    moved: &str,
    before: Option<&str>,
) -> Result<bool, StorageError> {
    let mut statement =
        connection.prepare_cached(&format!("SELECT 1 FROM {table} WHERE id = ?1 AND user_id = ?2"))?;
    for id in std::iter::once(moved).chain(before) {
        if statement.query_row(params![id, user.as_str()], |_| Ok(())).optional()?.is_none() {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Moves `moved` before `before`, or to the end, among `peers`, a
/// partition in its order, and writes the partition's new order to `table`;
/// false, writing nothing, when either row is not in the partition. Ids
/// compare in any case, as the index compares them.
fn write_order(
    connection: &Connection,
    table: &'static str,
    mut peers: Vec<String>,
    moved: &str,
    before: Option<&str>,
) -> Result<bool, StorageError> {
    let Some(from) = peers
        .iter()
        .position(|peer| peer.eq_ignore_ascii_case(moved))
    else {
        return Ok(false);
    };
    if before.is_some_and(|before| !peers.iter().any(|peer| peer.eq_ignore_ascii_case(before))) {
        return Ok(false);
    }
    let row = peers.remove(from);
    let to = match before {
        Some(before) => peers
            .iter()
            .position(|peer| peer.eq_ignore_ascii_case(before))
            // The row moves before itself: it stays where it was.
            .unwrap_or(from),
        None => peers.len(),
    };
    peers.insert(to, row);
    let mut statement =
        connection.prepare_cached(&format!("UPDATE {table} SET sort_order = ?1 WHERE id = ?2"))?;
    for (position, peer) in peers.iter().enumerate() {
        let position = i64::try_from(position).expect("a partition's size fits an i64");
        statement.execute(params![position, peer])?;
    }
    Ok(true)
}
