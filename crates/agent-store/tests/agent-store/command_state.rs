//! A node's command-state history (`command-state-history.md`): what a
//! store gives back is checked, never repaired.

use demi_agent_store::{
    BoundaryEdge, CommandStateHistory, CommandStateSnapshot, CommandStorageKey, SessionBoundary,
};
use serde_json::Value;

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
