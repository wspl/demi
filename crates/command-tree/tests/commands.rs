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

fn input(schema: Value) -> Result<demi_command_tree::InputSpec, String> {
    let schema: demi_command_tree::Schema = serde_json::from_value(schema).unwrap();
    demi_command_tree::InputSpec::from_schema(&schema).map_err(|error| error.to_string())
}

#[test]
fn the_input_subset_takes_scalars_enums_and_arrays_and_refuses_the_rest() {
    use demi_command_tree::FieldKind;

    let spec = input(json!({"title": "Args", "type": "object", "additionalProperties": false,
        "required": ["path"], "properties": {
            "path": {"type": "string", "minLength": 1, "description": "A file"},
            "count": {"type": "integer", "format": "uint32", "minimum": 0},
            "mode": {"type": "string", "enum": ["fast", "slow"]},
            "tags": {"type": "array", "items": {"type": "string", "pattern": "^[a-z]+$"}, "maxItems": 3},
            "force": {"type": "boolean"}}}))
    .unwrap();
    let fields: Vec<_> = spec
        .fields()
        .iter()
        .map(|field| (field.name.as_str(), field.kind.clone(), field.required))
        .collect();
    assert_eq!(fields, [
        ("path", FieldKind::String, true),
        ("count", FieldKind::Integer, false),
        ("mode", FieldKind::Enum(vec!["fast".into(), "slow".into()]), false),
        ("tags", FieldKind::Array(Box::new(FieldKind::String)), false),
        ("force", FieldKind::Boolean, false),
    ]);

    let refused = |field: Value| {
        input(json!({"type": "object", "additionalProperties": false,
            "properties": {"field": field}}))
        .unwrap_err()
    };
    for (field, reason) in [
        (json!({"type": "integer", "default": 2}), "carries a default"),
        (json!({"type": ["string", "null"]}), "allows null"),
        (json!({"type": "string", "enum": ["a", null]}), "allows null"),
        (json!({"type": "object", "properties": {"inner": {"type": "string"}}}), "nested object"),
        (json!({"oneOf": [{"type": "string"}, {"type": "integer"}]}), "union"),
        (json!({"$ref": "#"}), "refers to another schema"),
        (json!({"type": "array", "items": {"type": "array", "items": {"type": "string"}}}), "array of arrays"),
        (json!({"type": "array", "items": {"type": "string"}, "uniqueItems": true}), "\"uniqueItems\""),
        (json!({"type": "string", "format": "uri"}), "format"),
    ] {
        let error = refused(field);
        assert!(error.starts_with("input \"field\": "), "{error}");
        assert!(error.contains(reason), "{error} lacks {reason}");
    }
    let open = input(json!({"type": "object", "properties": {}})).unwrap_err();
    assert!(open.contains("additionalProperties"), "{open}");
}

/// The input of `demi example`.
#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
#[allow(dead_code, reason = "read only through its schema")]
struct ExampleArgs {
    /// The file to read
    path: String,
    /// How many times
    #[schemars(range(min = 1, max = 9))]
    count: Option<u32>,
    mode: Option<Mode>,
    tags: Vec<String>,
    labels: Option<Vec<String>>,
    no_cache: Option<bool>,
}

#[derive(serde::Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
#[allow(dead_code, reason = "read only through its schema")]
enum Mode {
    Fast,
    Slow,
}

#[test]
fn declared_argument_types_generate_schemas_inside_the_subset() {
    let schema = demi_command_tree::command_schema_settings()
        .into_generator()
        .into_root_schema_for::<ExampleArgs>();
    let value = schema.to_value();
    assert!(value.get("$schema").is_none());
    assert!(!value.to_string().contains("null"), "{value}");
    assert_eq!(value["required"], json!(["path", "tags"]));
    assert_eq!(value["properties"]["count"]["maximum"], json!(9));
    assert_eq!(value["properties"]["mode"]["enum"], json!(["fast", "slow"]));
    let spec = input(value).unwrap();
    assert_eq!(spec.fields().len(), 6);
}
