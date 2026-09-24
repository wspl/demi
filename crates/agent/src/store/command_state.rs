//! A node's command-state history (`command-state-history.md`): immutable
//! versions of its command storage and the boundaries that name the version
//! current at a point of the transcript. The session captures a boundary as
//! each block changes; history rewrites, Forks and command storage read and
//! write the versions.

use std::collections::{BTreeMap, BTreeSet};

use demi_core::{Block, BlockId};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A key of a node's command storage: a nonempty relative name with no NUL,
/// no leading `/` or drive letter, and no `..` segment.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CommandStorageKey(String);

impl CommandStorageKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for CommandStorageKey {
    type Error = CommandStateError;

    fn try_from(key: String) -> Result<Self, CommandStateError> {
        let bytes = key.as_bytes();
        let drive = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'/' | b'\\');
        let traverses = key.split(['/', '\\']).any(|segment| segment == "..");
        if key.is_empty() || key.contains('\0') || key.starts_with('/') || drive || traverses {
            return Err(CommandStateError(format!(
                "command storage key {key:?} is not a relative name without path traversal"
            )));
        }
        Ok(Self(key))
    }
}

impl From<CommandStorageKey> for String {
    fn from(key: CommandStorageKey) -> String {
        key.0
    }
}

/// Where a boundary sits relative to its block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoundaryEdge {
    /// Before a block that opens an input turn: the version current when the
    /// session started processing the turn. Recorded once.
    BeforeUser,
    /// When assistant text completed. Recorded once.
    AfterAssistant,
    /// When the block last changed; it moves forward with the block.
    AfterBlock,
}

/// One immutable version: the complete map of the node's keys.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandVersion {
    pub revision: u64,
    pub values: BTreeMap<CommandStorageKey, Value>,
}

/// The version that was current at one edge of one block.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionBoundary {
    pub block_id: BlockId,
    pub edge: BoundaryEdge,
    pub command_revision: u64,
}

/// A node's command state as its checkpoint carries it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CommandStateSnapshot {
    /// The current version.
    pub revision: u64,
    pub versions: Vec<CommandVersion>,
    pub boundaries: Vec<SessionBoundary>,
}

impl CommandStateSnapshot {
    /// A new node's state: the explicit empty version 0, current.
    pub fn initial() -> Self {
        Self {
            revision: 0,
            versions: vec![CommandVersion {
                revision: 0,
                values: BTreeMap::new(),
            }],
            boundaries: Vec::new(),
        }
    }
}

/// Why a command state is refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct CommandStateError(String);

type Values = BTreeMap<CommandStorageKey, Value>;

/// A session's live command state and whether its checkpoint must carry it
/// again.
#[derive(Debug)]
pub(crate) struct CommandStateHistory {
    versions: BTreeMap<u64, Values>,
    boundaries: BTreeMap<(BlockId, BoundaryEdge), u64>,
    current: u64,
    dirty: bool,
}

impl CommandStateHistory {
    /// A new node's history, which its first checkpoint already holds.
    pub(crate) fn new() -> Self {
        Self::restore(CommandStateSnapshot::initial()).expect("the initial command state is valid")
    }

    /// The history a checkpoint carries. Its rules across versions and
    /// boundaries are checked, never repaired: revisions are unique, the
    /// empty version 0 and the current version exist, each boundary is one
    /// per block and edge and names a version that exists.
    pub(crate) fn restore(snapshot: CommandStateSnapshot) -> Result<Self, CommandStateError> {
        let refuse = |reason: &str| {
            Err(CommandStateError(format!(
                "invalid command state: {reason}"
            )))
        };
        let mut versions = BTreeMap::new();
        for version in snapshot.versions {
            if versions.insert(version.revision, version.values).is_some() {
                return refuse("a revision appears twice");
            }
        }
        if !versions.get(&0).is_some_and(BTreeMap::is_empty) {
            return refuse("version 0 is missing or not empty");
        }
        if !versions.contains_key(&snapshot.revision) {
            return refuse("the current revision has no version");
        }
        let mut boundaries = BTreeMap::new();
        for boundary in snapshot.boundaries {
            if !versions.contains_key(&boundary.command_revision) {
                return refuse("a boundary names a revision that has no version");
            }
            let key = (boundary.block_id, boundary.edge);
            if boundaries.insert(key, boundary.command_revision).is_some() {
                return refuse("a block has two boundaries of one edge");
            }
        }
        Ok(Self {
            versions,
            boundaries,
            current: snapshot.revision,
            dirty: false,
        })
    }

    /// The current version's revision.
    pub(crate) fn revision(&self) -> u64 {
        self.current
    }

    /// The current version's keys and values.
    pub(crate) fn values(&self) -> &Values {
        self.versions
            .get(&self.current)
            .expect("the current revision has a version")
    }

    /// The version that holds `values`, one past the largest revision the
    /// node has used; none when `values` equal the current version's under
    /// RFC 8785 canonical JSON, where key order and a number's form do not
    /// matter.
    pub(crate) fn prepare(&self, values: Values) -> Option<CommandVersion> {
        if canonical(&values) == canonical(self.values()) {
            return None;
        }
        let revision = self
            .versions
            .keys()
            .next_back()
            .map_or(1, |largest| largest + 1);
        Some(CommandVersion { revision, values })
    }

    /// Makes a committed version current.
    pub(crate) fn accept(&mut self, version: CommandVersion) {
        self.current = version.revision;
        self.versions.insert(version.revision, version.values);
    }

    /// The revision recorded at `edge` of `block`.
    pub(crate) fn boundary(&self, block: &BlockId, edge: BoundaryEdge) -> Option<u64> {
        self.boundaries.get(&(block.clone(), edge)).copied()
    }

    /// The command state of a cut: the boundaries of the blocks `blocks`
    /// retains, with `revision` current, and every version, or only the
    /// versions those boundaries and `revision` refer to when
    /// `referenced_only`, as a Fork copies them.
    pub(crate) fn select(
        &self,
        blocks: &[Block],
        revision: u64,
        referenced_only: bool,
    ) -> CommandStateSnapshot {
        let retained: BTreeSet<&BlockId> = blocks.iter().map(Block::id).collect();
        let boundaries: Vec<SessionBoundary> = self
            .boundaries
            .iter()
            .filter(|((block, _), _)| retained.contains(block))
            .map(|((block_id, edge), revision)| SessionBoundary {
                block_id: block_id.clone(),
                edge: *edge,
                command_revision: *revision,
            })
            .collect();
        let mut referenced: BTreeSet<u64> = boundaries
            .iter()
            .map(|boundary| boundary.command_revision)
            .collect();
        referenced.extend([0, revision]);
        let versions = self
            .versions
            .iter()
            .filter(|(version, _)| !referenced_only || referenced.contains(version))
            .map(|(revision, values)| CommandVersion {
                revision: *revision,
                values: values.clone(),
            })
            .collect();
        CommandStateSnapshot {
            revision,
            versions,
            boundaries,
        }
    }

    /// Records that `revision` was current at `edge` of `block`. The
    /// `before_user` and `after_assistant` edges keep their first record; an
    /// `after_block` boundary moves forward with its block.
    pub(crate) fn capture(&mut self, block: BlockId, edge: BoundaryEdge, revision: u64) {
        debug_assert!(
            self.versions.contains_key(&revision),
            "a boundary names a known version"
        );
        let key = (block, edge);
        match self.boundaries.get(&key) {
            Some(recorded) if *recorded == revision => return,
            Some(_) if edge != BoundaryEdge::AfterBlock => return,
            _ => {}
        }
        self.boundaries.insert(key, revision);
        self.dirty = true;
    }

    pub(crate) fn has_boundary(&self, block: &BlockId, edge: BoundaryEdge) -> bool {
        self.boundaries.contains_key(&(block.clone(), edge))
    }

    /// Drops the boundaries of blocks the transcript no longer holds.
    pub(crate) fn retain_boundaries(&mut self, blocks: &[Block]) {
        let retained: BTreeSet<&BlockId> = blocks.iter().map(Block::id).collect();
        let before = self.boundaries.len();
        self.boundaries
            .retain(|(block, _), _| retained.contains(block));
        if self.boundaries.len() != before {
            self.dirty = true;
        }
    }

    /// The whole state, with `pending` current when a commit carries it.
    pub(crate) fn snapshot(&self, pending: Option<&CommandVersion>) -> CommandStateSnapshot {
        let mut versions: Vec<CommandVersion> = self
            .versions
            .iter()
            .map(|(revision, values)| CommandVersion {
                revision: *revision,
                values: values.clone(),
            })
            .collect();
        versions.extend(pending.cloned());
        CommandStateSnapshot {
            revision: pending.map_or(self.current, |pending| pending.revision),
            versions,
            boundaries: self
                .boundaries
                .iter()
                .map(|((block_id, edge), revision)| SessionBoundary {
                    block_id: block_id.clone(),
                    edge: *edge,
                    command_revision: *revision,
                })
                .collect(),
        }
    }

    /// The state for the next save, when it changed since the last one or a
    /// commit carries `pending`.
    pub(crate) fn take_update(
        &mut self,
        pending: Option<&CommandVersion>,
    ) -> Option<CommandStateSnapshot> {
        if !self.dirty && pending.is_none() {
            return None;
        }
        self.dirty = false;
        Some(self.snapshot(pending))
    }

    /// A save that carried the state failed: the next one carries it again.
    pub(crate) fn mark_dirty(&mut self) {
        self.dirty = true;
    }
}

/// A command value map as RFC 8785 canonical JSON.
fn canonical(values: &Values) -> Vec<u8> {
    serde_json_canonicalizer::to_vec(values).expect("a JSON map serializes")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn boundary(block: &str, edge: BoundaryEdge, revision: u64) -> SessionBoundary {
        SessionBoundary {
            block_id: block.try_into().unwrap(),
            edge,
            command_revision: revision,
        }
    }

    #[test]
    fn a_stored_history_is_refused_rather_than_repaired() {
        let mut duplicate = CommandStateSnapshot::initial();
        duplicate.boundaries = vec![
            boundary("b1", BoundaryEdge::AfterBlock, 0),
            boundary("b1", BoundaryEdge::AfterBlock, 0),
        ];
        let mut dangling = CommandStateSnapshot::initial();
        dangling.boundaries = vec![boundary("b1", BoundaryEdge::BeforeUser, 3)];
        let mut current = CommandStateSnapshot::initial();
        current.revision = 2;
        let mut no_empty = CommandStateSnapshot::initial();
        no_empty.versions[0]
            .values
            .insert("todos.json".to_owned().try_into().unwrap(), Value::Null);
        for invalid in [duplicate, dangling, current, no_empty] {
            assert!(CommandStateHistory::restore(invalid).is_err());
        }
        for key in ["", "/etc", "C:\\x", "a/../b", "a\0b"] {
            assert!(
                CommandStorageKey::try_from(key.to_owned()).is_err(),
                "{key:?}"
            );
        }
        assert!(CommandStorageKey::try_from("todos.json".to_owned()).is_ok());
    }

    #[test]
    fn a_capture_that_changes_nothing_makes_no_save_due() {
        let mut history = CommandStateHistory::new();
        let block = BlockId::try_from("b1").unwrap();
        history.capture(block.clone(), BoundaryEdge::AfterBlock, 0);
        history.capture(block.clone(), BoundaryEdge::BeforeUser, 0);
        let update = history.take_update(None).unwrap();
        assert_eq!(update.boundaries.len(), 2);
        // Unchanged: no update is due.
        history.capture(block, BoundaryEdge::BeforeUser, 0);
        assert_eq!(history.take_update(None), None);
    }
}
