//! The `demi.file` package's contract: what its operations accept and refuse.

use demi_command_package_file_protocol::{OPERATIONS, Operation, OperationError};
use serde_json::{Value, json};

#[test]
fn file_arguments_refuse_empty_old_text_and_zero_positions() {
    let edit = |args: Value| Operation::parse("file.edit", args);
    assert!(edit(json!({"path": "a", "old": "x", "new": "y", "occurrence": 2})).is_ok());
    for invalid in [
        json!({"path": "a", "old": "", "new": "y"}),
        json!({"path": "a", "old": "x", "new": "y", "occurrence": 0}),
        json!({"path": "a", "old": "x", "new": "y", "context": 0}),
        json!({"path": "a", "old": "x", "new": "y", "context": null}),
        json!({"path": "a", "old": "x"}),
    ] {
        assert!(matches!(edit(invalid.clone()), Err(OperationError::Invalid(_))), "{invalid}");
    }
    assert!(matches!(
        Operation::parse("file.read", json!({"path": "a", "extra": true})),
        Err(OperationError::Invalid(_))
    ));
}

/// Every name the descriptor lists decodes, so it lists nothing unserved,
/// and a name outside the package is unknown to it.
#[test]
fn the_listed_operations_are_the_ones_the_package_decodes() {
    assert_eq!(OPERATIONS, ["file.read", "file.create", "file.edit", "file.patch"]);
    for name in OPERATIONS {
        assert!(!matches!(Operation::parse(name, json!({})), Err(OperationError::Unknown(_))), "{name}");
    }
    for other in ["file.remove", "browser.tabs"] {
        assert!(
            matches!(Operation::parse(other, json!({})), Err(OperationError::Unknown(name)) if name == other),
            "{other}"
        );
    }
}
