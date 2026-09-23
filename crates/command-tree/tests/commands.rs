//! Declarations, help and command lines against the cases the TypeScript
//! command loader shares, and the declaration rules' refusals.

use demi_command_tree::Node;
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../../packages/command-loader/src/fixtures/cli.json"
    ))
    .unwrap()
}

fn fixture_tree() -> Node {
    serde_json::from_value(fixture()["manifest"]["roots"]["fixture"]["tree"].clone()).unwrap()
}

#[test]
fn help_and_command_lines_match_the_shared_cases() {
    let fixture = fixture();
    let tree = fixture_tree();
    tree.validate().unwrap();
    assert_eq!(tree.help("fixture"), fixture["help"].as_str().unwrap());
    for case in fixture["cases"].as_array().unwrap() {
        let argv: Vec<String> = serde_json::from_value(case["argv"].clone()).unwrap();
        let result = (|| {
            let selected = tree.select(&argv)?;
            let parsed = selected.parse(&argv)?;
            if parsed.help {
                return Ok(parsed);
            }
            parsed.validate(
                selected.node.leaf().unwrap(),
                Some(case["stdin"].as_str().unwrap().into()),
            )
        })();
        if case["invalid"] == true {
            assert!(result.is_err(), "argv={argv:?}");
        } else {
            assert_eq!(
                serde_json::to_value(result.unwrap()).unwrap(),
                case["parsed"],
                "argv={argv:?}"
            );
        }
    }
}

#[test]
fn a_leaf_runs_one_way_and_names_valid_inputs() {
    let rpc = json!({"name": "add", "summary": "Add", "kind": "rpc"});
    assert!(serde_json::from_value::<Node>(rpc.clone()).is_ok());
    let mut bound = rpc.clone();
    bound["binding"] = json!({"package": "demi.builtin", "operation": "file.read",
        "descriptorHash": "a".repeat(64)});
    assert!(serde_json::from_value::<Node>(bound).is_err());
    let unbound = json!({"name": "read", "summary": "Read", "kind": "native"});
    assert!(serde_json::from_value::<Node>(unbound).is_err());

    let declaration = |input: Value| {
        let mut leaf = rpc.clone();
        leaf["input"] = input;
        serde_json::from_value::<Node>(leaf).unwrap().validate()
    };
    assert!(declaration(json!({"type": "object", "properties": {"text": {"type": "string"}}})).is_ok());
    assert!(declaration(json!({"type": "array"})).is_err());
    assert!(declaration(json!({"type": "object", "properties": {"json": {"type": "boolean"}}})).is_err());
    assert!(declaration(json!({"type": "object", "properties": {"bad name": {"type": "string"}}})).is_err());
}

#[test]
fn groups_name_distinct_subcommands() {
    let leaf = json!({"name": "add", "summary": "Add", "kind": "rpc"});
    let group = |subcommands: Value| {
        serde_json::from_value::<Node>(json!({"name": "todo", "summary": "Todos",
            "subcommands": subcommands}))
        .unwrap()
        .validate()
    };
    assert!(group(json!([leaf.clone()])).is_ok());
    assert!(group(json!([leaf.clone(), leaf])).is_err());
    assert!(group(json!([])).is_err());
}
