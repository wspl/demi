//! The JSON Schemas the browser's validators are generated from say what
//! serde does: an optional field may be absent but never null, a nullable
//! field is always present and may be null, a type the backend receives
//! refuses unknown fields, and bounds come from the garde attributes.

use demi_core::{Block, MAX_SAFE_INTEGER, PendingSteer};
use schemars::{SchemaGenerator, generate::SchemaSettings};
use serde_json::{Value, json};

/// The schema of `T` with its definitions, under both of schemars' contracts:
/// what serde accepts and what it writes.
fn schemas<T: schemars::JsonSchema>() -> [Value; 2] {
    [SchemaSettings::default(), SchemaSettings::default().for_serialize()].map(|settings| {
        let schema = SchemaGenerator::new(settings).into_root_schema_for::<T>();
        serde_json::to_value(schema).unwrap()
    })
}

/// The definition `name` in `root`, or `root` itself when it is that type.
fn definition<'a>(root: &'a Value, name: &str) -> &'a Value {
    if root["title"] == name {
        return root;
    }
    root["$defs"]
        .get(name)
        .unwrap_or_else(|| panic!("no definition {name}"))
}

/// The variant of a tagged enum's schema whose `tag` is `value`.
fn variant<'a>(schema: &'a Value, tag: &str, value: &str) -> &'a Value {
    schema["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .find(|variant| variant["properties"][tag]["const"] == value)
        .unwrap_or_else(|| panic!("no variant {tag} = {value}"))
}

fn required(schema: &Value) -> Vec<&str> {
    schema["required"]
        .as_array()
        .map(|names| names.iter().map(|name| name.as_str().unwrap()).collect())
        .unwrap_or_default()
}

fn admits_null(schema: &Value) -> bool {
    let typed = schema["type"]
        .as_array()
        .is_some_and(|types| types.contains(&json!("null")));
    let any_of = schema["anyOf"]
        .as_array()
        .is_some_and(|options| options.contains(&json!({"type": "null"})));
    typed || any_of
}

#[test]
fn optional_fields_may_be_absent_and_nullable_fields_may_be_null() {
    for root in schemas::<Block>() {
        let user = variant(&root, "type", "user");
        assert!(required(user).contains(&"preamble"));
        assert!(admits_null(&user["properties"]["preamble"]));

        let content = definition(&root, "UserContentBlock");
        let attachment = variant(content, "type", "attachment");
        assert!(!required(attachment).contains(&"snippet"));
        assert_eq!(attachment["properties"]["snippet"]["type"], "string");

        let selection = definition(&root, "ModelSelection");
        for field in ["thinking", "serviceTierId"] {
            assert!(required(selection).contains(&field), "{field}");
            assert!(admits_null(&selection["properties"][field]), "{field}");
        }

        let error = variant(&root, "type", "error");
        assert!(required(error).contains(&"code"));
        assert!(!required(error).contains(&"diagnostics"));
        assert!(!admits_null(&error["properties"]["diagnostics"]));

        let text = variant(&root, "type", "text");
        assert!(!required(text).contains(&"forkable"));
    }
}

#[test]
fn received_types_are_strict_and_types_only_the_browser_receives_are_tolerant() {
    let [root, _] = schemas::<Block>();
    for kind in ["user", "tool_call", "abort"] {
        assert_eq!(variant(&root, "type", kind)["additionalProperties"], false, "{kind}");
    }
    let disabled = variant(definition(&root, "ThinkingConfig"), "type", "disabled");
    assert_eq!(disabled["additionalProperties"], false);

    let [steer, _] = schemas::<PendingSteer>();
    assert!(steer.get("additionalProperties").is_none());
}

#[test]
fn bytes_times_identities_and_bounds_carry_their_formats() {
    let [root, _] = schemas::<Block>();
    let user = variant(&root, "type", "user");
    assert_eq!(user["properties"]["createdAt"]["format"], "date-time");
    assert_eq!(user["properties"]["id"]["minLength"], 1);

    let binary = variant(definition(&root, "MediaSource"), "type", "binary");
    assert_eq!(binary["properties"]["data"]["contentEncoding"], "base64");
    let reference = variant(definition(&root, "MediaSource"), "type", "ref");
    assert_eq!(reference["properties"]["ref"]["pattern"], "^[0-9a-f]{64}$");

    let attachment = variant(definition(&root, "UserContentBlock"), "type", "attachment");
    assert_eq!(attachment["properties"]["name"]["minLength"], 1);
    assert_eq!(attachment["properties"]["sizeBytes"]["maximum"], MAX_SAFE_INTEGER);

    let model = definition(&root, "Model");
    assert_eq!(model["properties"]["outputLimit"]["minimum"], 1);
    let view = variant(definition(&root, "ToolView"), "kind", "shell");
    assert_eq!(view["properties"]["runningMs"]["maximum"], MAX_SAFE_INTEGER);
}
