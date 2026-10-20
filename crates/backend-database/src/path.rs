//! A node's current path through its tree of rows.
//!
//! The path is the node's leaf, and the runs of consecutive row numbers from its
//! first block to the leaf, which `path_runs` keeps so a range of positions reads
//! a range of rows. The runs are derived from the rows and the leaf, and written
//! with the leaf in the same transaction.

use std::collections::BTreeMap;
use std::ops::Range;

use demi_agent_store::versions::{Sibling, version_rows, versions_of};
use demi_conversation_socket_protocol::Versions;
use rusqlite::{Connection, OptionalExtension, Transaction, params};

use super::StorageError;
use super::columns::decode;

/// The rows of the current path of the node `run.node_id`, as the `FROM` of
/// a query that names them `run` and `blocks`, each at the position
/// [`POSITION`]. The runs come first, so a query ordered by `run.start`
/// and `blocks.idx` reads them in the path's order.
pub(crate) const PATH_ROWS: &str = "path_runs AS run CROSS JOIN blocks
  ON blocks.node_id = run.node_id AND blocks.idx >= run.first AND blocks.idx < run.first + run.length";

/// The position on the path of a row of [`PATH_ROWS`].
pub(crate) const POSITION: &str = "(run.start + blocks.idx - run.first)";

/// The length of the current path of the node `nodes.id`.
pub(crate) const PATH_LENGTH: &str =
    "(SELECT coalesce(max(own.start + own.length), 0) FROM path_runs AS own WHERE own.node_id = nodes.id)";

/// One run: the rows `first` to `first + length - 1` at the positions from
/// `start`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Run {
    start: i64,
    first: i64,
    length: i64,
}

impl Run {
    fn end(&self) -> i64 {
        self.start + self.length
    }
}

/// A node's current path as its runs, read to map positions to rows and
/// changed in memory by a save, which then writes the runs it changed.
#[derive(Debug, Clone)]
pub(crate) struct Path {
    node: String,
    runs: Vec<Run>,
    /// The start of the first run the path changed since it was read.
    changed_from: Option<i64>,
}

impl Path {
    /// The node's path as its runs hold it; runs that do not join are
    /// corrupt data.
    pub(crate) fn read(connection: &Connection, node: &str) -> Result<Self, StorageError> {
        let mut statement =
            connection.prepare_cached("SELECT start, first, length FROM path_runs WHERE node_id = ?1 ORDER BY start")?;
        let mut rows = statement.query([node])?;
        let mut runs: Vec<Run> = Vec::new();
        while let Some(row) = rows.next()? {
            let run = Run {
                start: row.get(0)?,
                first: row.get(1)?,
                length: row.get(2)?,
            };
            if run.start != runs.last().map_or(0, Run::end) {
                return Err(StorageError::Corrupt {
                    table: "path_runs",
                    column: "start",
                    reason: format!("the path of node {node} has a gap before position {}", run.start),
                });
            }
            runs.push(run);
        }
        Ok(Self {
            node: node.to_owned(),
            runs,
            changed_from: None,
        })
    }

    /// How many blocks the path holds.
    pub(crate) fn len(&self) -> i64 {
        self.runs.last().map_or(0, Run::end)
    }

    /// The row at `position`.
    pub(crate) fn row_at(&self, position: i64) -> Option<i64> {
        let run = self.runs.iter().find(|run| run.start <= position && position < run.end())?;
        Some(run.first + position - run.start)
    }

    /// The position of `row`, when the path holds it.
    pub(crate) fn position_of(&self, row: i64) -> Option<i64> {
        let run = self
            .runs
            .iter()
            .find(|run| run.first <= row && row < run.first + run.length)?;
        Some(run.start + row - run.first)
    }

    /// The leaf: the row at the path's last position.
    pub(crate) fn leaf(&self) -> Option<i64> {
        self.runs.last().map(|run| run.first + run.length - 1)
    }

    /// The rows at the positions `range`, each run's as a range of rows
    /// after the position of its first.
    pub(crate) fn segments(&self, range: Range<i64>) -> Vec<(i64, Range<i64>)> {
        self.runs
            .iter()
            .filter(|run| run.start < range.end && range.start < run.end())
            .map(|run| {
                let from = range.start.max(run.start);
                let to = range.end.min(run.end());
                (from, run.first + from - run.start..run.first + to - run.start)
            })
            .collect()
    }

    /// Leaves the positions from `at` on.
    pub(crate) fn truncate(&mut self, at: i64) {
        if at >= self.len() {
            return;
        }
        self.runs.retain(|run| run.start < at);
        if let Some(last) = self.runs.last_mut() {
            last.length = last.length.min(at - last.start);
        }
        self.changed(at.min(self.runs.last().map_or(0, |run| run.start)));
    }

    /// Adds `row` at the path's end.
    pub(crate) fn push(&mut self, row: i64) {
        let end = self.len();
        match self.runs.last_mut() {
            Some(last) if last.first + last.length == row => {
                last.length += 1;
                let start = last.start;
                self.changed(start);
            }
            _ => {
                self.runs.push(Run {
                    start: end,
                    first: row,
                    length: 1,
                });
                self.changed(end);
            }
        }
    }

    fn changed(&mut self, start: i64) {
        self.changed_from = Some(self.changed_from.map_or(start, |from| from.min(start)));
    }

    /// Writes the runs this path changed, and the node's leaf.
    pub(crate) fn write(&self, transaction: &Transaction<'_>) -> Result<(), StorageError> {
        let Some(from) = self.changed_from else {
            return Ok(());
        };
        transaction
            .prepare_cached("DELETE FROM path_runs WHERE node_id = ?1 AND start >= ?2")?
            .execute(params![self.node, from])?;
        let mut insert = transaction
            .prepare_cached("INSERT INTO path_runs (node_id, start, first, length) VALUES (?1, ?2, ?3, ?4)")?;
        for run in self.runs.iter().filter(|run| run.start >= from) {
            insert.execute(params![self.node, run.start, run.first, run.length])?;
        }
        transaction
            .prepare_cached("UPDATE nodes SET leaf = ?2 WHERE id = ?1")?
            .execute(params![self.node, self.leaf()])?;
        Ok(())
    }

    /// The versions at each position in `range` that holds more than one.
    ///
    /// A place holds versions only where a row's parent has another row after it,
    /// which the index of rows that do not follow the row before them finds without
    /// reading the path's rows.
    ///
    /// A block a compaction carried past its boundary shows the versions of the
    /// place it was carried from, which the boundary then stands at without showing
    /// them, unless its new place holds versions of its own.
    pub(crate) fn versions(
        &self,
        connection: &Connection,
        range: Range<i64>,
    ) -> Result<BTreeMap<i64, Versions>, StorageError> {
        let mut statement = connection.prepare_cached(
            "SELECT DISTINCT follows FROM blocks INDEXED BY blocks_branching
             WHERE node_id = ?1 AND follows IS NOT idx - 1",
        )?;
        let parents = statement
            .query_map([&self.node], |row| row.get::<_, Option<i64>>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut versions = BTreeMap::new();
        for parent in parents {
            let Some(position) = self.place_of(parent) else {
                continue;
            };
            let Some(row) = self.row_at(position) else {
                continue;
            };
            if let Some(held) = versions_of(&siblings(connection, &self.node, parent)?, row as u64) {
                versions.insert(position, held);
            }
        }
        let mut statement = connection.prepare_cached(
            "SELECT carrier.idx FROM blocks AS carrier INDEXED BY blocks_carrying
               JOIN blocks AS carried ON carried.node_id = carrier.node_id AND carried.idx = carrier.replaces
             WHERE carrier.node_id = ?1 AND carried.follows IS NOT carrier.follows",
        )?;
        let carriers = statement
            .query_map([&self.node], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        for carrier in carriers {
            let Some(position) = self.position_of(carrier) else {
                continue;
            };
            if versions.contains_key(&position) {
                continue;
            }
            let origin = carried(connection, &self.node, carrier)?;
            let parent = links(connection, &self.node, origin)?.0;
            if let Some(held) = versions_of(&siblings(connection, &self.node, parent)?, origin as u64) {
                if let Some(standing) = self.place_of(parent) {
                    versions.remove(&standing);
                }
                versions.insert(position, held);
            }
        }
        versions.retain(|position, _| range.contains(position));
        Ok(versions)
    }

    /// The position of the place after `parent` on this path, the first
    /// for none; none when the path does not hold `parent`.
    fn place_of(&self, parent: Option<i64>) -> Option<i64> {
        match parent {
            None => Some(0),
            Some(parent) => self.position_of(parent).map(|position| position + 1),
        }
    }

    /// Makes the latest leaf below the `version`th version at `position` the
    /// path's end, and answers the position the path changed from.
    ///
    /// That position is the place of the versions `position` shows, which for a
    /// block a compaction carried is the place it was carried from. None when the
    /// path does not reach `position` or its place holds no such version.
    pub(crate) fn switch(
        &mut self,
        connection: &Connection,
        position: i64,
        version: u32,
    ) -> Result<Option<i64>, StorageError> {
        let Some(row) = self.row_at(position) else {
            return Ok(None);
        };
        let shown = self.versions(connection, position..position + 1)?;
        let own = match position {
            0 => None,
            _ => self.row_at(position - 1),
        };
        // The place the versions shown at `position` are of: its own, or
        // the one its block was carried from.
        let parent = if shown.is_empty() || self.versions_here(connection, own, row)? {
            own
        } else {
            links(connection, &self.node, carried(connection, &self.node, row)?)?.0
        };
        let Some(from) = self.place_of(parent) else {
            return Ok(None);
        };
        let siblings = siblings(connection, &self.node, parent)?;
        let Some(members) = version_rows(&siblings, version) else {
            return Ok(None);
        };
        let members = serde_json::to_string(&members).expect("row numbers serialize");
        // Every row below the version's, and the latest of them, a leaf:
        // a row follows an older one.
        let leaf: i64 = connection.query_row(
            "WITH RECURSIVE below (idx) AS (
               SELECT value FROM json_each(?2)
               UNION ALL
               SELECT blocks.idx FROM blocks INDEXED BY blocks_by_follows
                 JOIN below ON blocks.node_id = ?1 AND blocks.follows = below.idx
             )
             SELECT max(idx) FROM below",
            params![self.node, members],
            |row| row.get(0),
        )?;
        let mut statement = connection.prepare_cached(
            "WITH RECURSIVE up (idx, follows) AS (
               SELECT idx, follows FROM blocks WHERE node_id = ?1 AND idx = ?2
               UNION ALL
               SELECT blocks.idx, blocks.follows FROM blocks JOIN up
                 ON blocks.node_id = ?1 AND blocks.idx = up.follows AND up.follows IS NOT ?3
             )
             SELECT idx FROM up ORDER BY idx",
        )?;
        let tail = statement
            .query_map(params![self.node, leaf, parent], |row| row.get::<_, i64>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        self.truncate(from);
        for row in tail {
            self.push(row);
        }
        Ok(Some(from))
    }

    /// Whether the place after `parent` holds versions with `row` among them.
    fn versions_here(&self, connection: &Connection, parent: Option<i64>, row: i64) -> Result<bool, StorageError> {
        Ok(versions_of(&siblings(connection, &self.node, parent)?, row as u64).is_some())
    }
}

/// The row `row` was carried from, through every compaction that carried
/// it: a row that replaces one with another parent is that one's block on a
/// new branch; `row` itself otherwise.
fn carried(connection: &Connection, node: &str, row: i64) -> Result<i64, StorageError> {
    let mut row = row;
    loop {
        let (follows, replaces) = links(connection, node, row)?;
        let Some(replaced) = replaces else {
            return Ok(row);
        };
        if links(connection, node, replaced)?.0 == follows {
            return Ok(row);
        }
        row = replaced;
    }
}

/// The row `row` follows and the one it replaces.
fn links(connection: &Connection, node: &str, row: i64) -> Result<(Option<i64>, Option<i64>), StorageError> {
    Ok(connection
        .prepare_cached("SELECT follows, replaces FROM blocks WHERE node_id = ?1 AND idx = ?2")?
        .query_row(params![node, row], |found| Ok((found.get(0)?, found.get(1)?)))?)
}

/// The rows of `node` that follow `parent`, none for a path's first rows,
/// with the rows they replace.
fn siblings(connection: &Connection, node: &str, parent: Option<i64>) -> Result<Vec<Sibling>, StorageError> {
    let mut statement = connection.prepare_cached(
        "SELECT idx, replaces FROM blocks INDEXED BY blocks_by_follows WHERE node_id = ?1 AND follows IS ?2 ORDER BY idx",
    )?;
    let rows = statement
        .query_map(params![node, parent], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, Option<i64>>(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    rows.into_iter()
        .map(|(row, replaces)| {
            Ok(Sibling {
                row: decode("blocks", "idx", u64::try_from(row))?,
                replaces: replaces
                    .map(|replaced| decode("blocks", "replaces", u64::try_from(replaced)))
                    .transpose()?,
            })
        })
        .collect()
}

/// The next row number of `node`: one past its latest row's.
pub(crate) fn next_row(connection: &Connection, node: &str) -> Result<i64, StorageError> {
    let next: Option<i64> = connection
        .query_row("SELECT max(idx) + 1 FROM blocks WHERE node_id = ?1", [node], |row| row.get(0))
        .optional()?
        .flatten();
    Ok(next.unwrap_or(0))
}
