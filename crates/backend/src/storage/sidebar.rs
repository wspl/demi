//! The sidebar's explicit order (`storage.md` § Control records): a move of
//! one row before another, or to the end, within its partition, which the
//! move writes whole in one transaction. Activity never reorders a row.

use demi_web_api::ids::{ConversationId, UserId, WorkspaceId};
use rusqlite::{Connection, OptionalExtension, params};

use super::StorageError;
use super::control::ControlService;

impl ControlService {
    /// Moves the user's conversation `moved` before `before`, or to the end,
    /// within its partition (`storage.md` § Control records): the
    /// conversations that are not archived, pinned as it is, and in the same
    /// workspace or in none. False, writing nothing, when either is not in
    /// that partition, an archived one included.
    pub(crate) async fn reorder_conversations(
        &self,
        user: UserId,
        moved: ConversationId,
        before: Option<ConversationId>,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let partition = transaction
                .query_row(
                    "SELECT archived, pinned, target_workspace_id FROM conversations WHERE id = ?1 AND user_id = ?2",
                    params![moved.as_str(), user.as_str()],
                    |row| Ok((row.get::<_, bool>(0)?, row.get::<_, bool>(1)?, row.get::<_, Option<String>>(2)?)),
                )
                .optional()?;
            let Some((false, pinned, workspace)) = partition else {
                return Ok(false);
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
            let before = before.as_ref().map(ConversationId::as_str);
            if !write_order(&transaction, "conversations", peers, moved.as_str(), before)? {
                return Ok(false);
            }
            transaction.commit()?;
            Ok(true)
        })
        .await
    }

    /// Moves the user's workspace `moved` before `before`, or to the end;
    /// false when either is not one of the user's workspaces.
    pub(crate) async fn reorder_workspaces(
        &self,
        user: UserId,
        moved: WorkspaceId,
        before: Option<WorkspaceId>,
    ) -> Result<bool, StorageError> {
        self.call(move |connection, _| {
            let transaction = connection.transaction()?;
            let peers = {
                let mut statement =
                    transaction.prepare_cached("SELECT id FROM workspaces WHERE user_id = ?1 ORDER BY sort_order, id")?;
                let rows = statement.query_map([user.as_str()], |row| row.get::<_, String>(0))?;
                rows.collect::<Result<Vec<String>, _>>()?
            };
            let before = before.as_ref().map(WorkspaceId::as_str);
            if !write_order(&transaction, "workspaces", peers, moved.as_str(), before)? {
                return Ok(false);
            }
            transaction.commit()?;
            Ok(true)
        })
        .await
    }
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
    let Some(from) = peers.iter().position(|peer| peer.eq_ignore_ascii_case(moved)) else {
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
    let mut statement = connection.prepare_cached(&format!("UPDATE {table} SET sort_order = ?1 WHERE id = ?2"))?;
    for (position, peer) in peers.iter().enumerate() {
        let position = i64::try_from(position).expect("a partition's size fits an i64");
        statement.execute(params![position, peer])?;
    }
    Ok(true)
}
