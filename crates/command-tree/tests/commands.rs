//! Declarations, help and command lines: the cases the TypeScript command
//! loader shares, how a command line fills each field and is refused, and
//! the declaration rules' refusals.

use demi_command_tree::{Node, Parsed};
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

/// What a command line comes to, as the runner reads it: `argv` after the
/// root's name, and the body read from stdin for a leaf that takes one.
fn read(tree: &Node, argv: &[&str], stdin: Option<&str>) -> Result<Parsed, String> {
    let argv: Vec<String> = argv.iter().map(|token| (*token).to_owned()).collect();
    let selected = tree.select(&argv).map_err(|error| error.to_string())?;
    let parsed = selected.parse(&argv).map_err(|error| error.to_string())?;
    match selected.node.leaf() {
        Some(leaf) if !parsed.help => parsed
            .validate(leaf, stdin.map(str::to_owned))
            .map_err(|error| error.to_string()),
        _ => Ok(parsed),
    }
}

/// The values a command line fills.
fn values(tree: &Node, argv: &[&str], stdin: Option<&str>) -> Value {
    let parsed = read(tree, argv, stdin).unwrap_or_else(|error| panic!("{argv:?}: {error}"));
    Value::Object(parsed.values)
}

/// Why a command line is refused.
fn refusal(tree: &Node, argv: &[&str], stdin: Option<&str>) -> String {
    read(tree, argv, stdin)
        .err()
        .unwrap_or_else(|| panic!("{argv:?} is accepted"))
}

#[test]
fn help_and_command_lines_match_the_shared_cases() {
    let fixture = fixture();
    let tree = fixture_tree();
    tree.validate().unwrap();
    assert_eq!(tree.help("fixture"), fixture["help"].as_str().unwrap());
    for case in fixture["cases"].as_array().unwrap() {
        let argv: Vec<&str> = case["argv"]
            .as_array()
            .unwrap()
            .iter()
            .map(|token| token.as_str().unwrap())
            .collect();
        let result = read(&tree, &argv, case["stdin"].as_str());
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

/// A leaf's input: an object with `properties` that requires `required` and
/// allows nothing else.
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required,
        "additionalProperties": false})
}

/// Commands with fields from every source, after the TypeScript shell's
/// command tests.
fn filer() -> Node {
    let tree: Node = serde_json::from_value(json!({
        "name": "filer", "summary": "Create, edit, and list files.", "subcommands": [
            {"name": "create", "summary": "Create a file.", "kind": "rpc",
                "input": object(json!({"path": {"type": "string"},
                    "content": {"type": "string", "maxLength": 8, "description": "File content"}}),
                    &["path", "content"]),
                "positionals": ["path"], "stdinField": "content"},
            {"name": "edit", "summary": "Replace text in a file.", "kind": "rpc",
                "input": object(json!({"path": {"type": "string"}, "old": {"type": "string"},
                    "new": {"type": "string"}, "occurrence": {"type": "integer", "minimum": 1}}),
                    &["path", "old", "new"]),
                "positionals": ["path"]},
            {"name": "measure", "summary": "Record readings.", "kind": "rpc",
                "input": object(json!({"v": {"type": "array", "items": {"type": "number"}},
                    "label": {"type": "array", "items": {"type": "string"}},
                    "quiet": {"type": "boolean"}}), &["v"])},
            {"name": "forward", "summary": "Forward argv.", "kind": "rpc",
                "input": object(json!({"args": {"type": "array", "items": {"type": "string"}}}),
                    &["args"]),
                "restField": "args"},
            {"name": "status", "summary": "Set a status.", "kind": "rpc",
                "input": object(json!({"status": {"type": "string",
                    "enum": ["pending", "in_progress", "done"]}}), &[])},
            {"name": "list", "summary": "List files.", "kind": "rpc",
                "output": {"json": object(json!({"files": {"type": "array",
                    "items": {"type": "string"}}}), &["files"])}},
            {"name": "watch", "summary": "Background pollers.", "subcommands": [
                {"name": "get", "summary": "Read a poller.", "kind": "rpc",
                    "input": object(json!({"id": {"type": "string"}}), &["id"]),
                    "positionals": ["id"]}]}]
    }))
    .unwrap();
    tree.validate().unwrap();
    tree
}

#[test]
fn each_field_takes_its_value_from_its_one_source() {
    let filer = filer();
    assert_eq!(
        values(&filer, &["create", "note.txt"], Some("body")),
        json!({"path": "note.txt", "content": "body"})
    );
    // A stdin body has no option form, even beside a body.
    for option in [
        &["--content"][..],
        &["--content", "inline"],
        &["--content=inline"],
    ] {
        let argv = [&["create", "note.txt"][..], option].concat();
        let error = refusal(&filer, &argv, Some("body"));
        assert!(
            error.starts_with("\"filer create\" reads content only from stdin. Remove --content"),
            "{error}"
        );
    }
    // A positional has no option form, and a standalone -- ends the options.
    assert_eq!(
        refusal(&filer, &["create", "note.txt", "inline"], Some("body")),
        "Unexpected positional argument \"inline\""
    );
    let error = refusal(&filer, &["create", "--path", "note.txt"], Some("body"));
    assert!(
        error.starts_with("\"path\" is a positional argument for \"filer create\""),
        "{error}"
    );
    assert_eq!(
        values(&filer, &["create", "--", "--help"], Some("body"))["path"],
        "--help"
    );
    // A rest field takes the tokens after -- as they are, and only those.
    assert_eq!(
        values(&filer, &["forward", "--", "--help", "--json"], None),
        json!({"args": ["--help", "--json"]})
    );
    let error = refusal(&filer, &["forward", "--args", "value"], None);
    assert!(
        error.starts_with("\"args\" is passed after -- for \"filer forward\""),
        "{error}"
    );
    // Help shows each field in its source's form, never a body as an option.
    let help = filer.help("filer");
    assert!(
        help.contains("  filer create <path> <<'EOF'\n  <content>\n  EOF\n"),
        "{help}"
    );
    assert!(
        help.contains("    Stdin body: content - File content"),
        "{help}"
    );
    assert!(help.contains("  filer forward -- <args>...\n"), "{help}");
    for option in ["--path", "--content", "--args"] {
        assert!(!help.contains(option), "{option} in {help}");
    }
}

#[test]
fn an_option_value_never_swallows_the_next_option() {
    let filer = filer();
    assert_eq!(
        refusal(
            &filer,
            &["edit", "note.txt", "--old", "--new", "replacement"],
            None
        ),
        "Missing value for \"--old\""
    );
    // --name=value passes a value that begins with --, or an empty one.
    assert_eq!(
        values(
            &filer,
            &["edit", "note.txt", "--old=--help", "--new="],
            None
        ),
        json!({"path": "note.txt", "old": "--help", "new": ""})
    );
    assert_eq!(
        refusal(
            &filer,
            &["edit", "note.txt", "--old", "a", "--old", "b", "--new", "c"],
            None
        ),
        "Duplicate value for \"old\""
    );
}

#[test]
fn argv_text_becomes_the_value_its_field_declares_and_one_refusal_names_every_failure() {
    let filer = filer();
    // Each element of a repeated option converts on its own.
    assert_eq!(
        values(&filer, &["measure", "--v", "12", "--v", "13"], None),
        json!({"v": [12, 13]})
    );
    assert_eq!(
        values(
            &filer,
            &["measure", "--v", "1.5", "--label", "a", "--quiet"],
            None
        ),
        json!({"v": [1.5], "label": ["a"], "quiet": true})
    );
    assert_eq!(
        values(&filer, &["measure", "--v", "1", "--quiet=false"], None)["quiet"],
        false
    );
    assert_eq!(
        values(
            &filer,
            &["edit", "f", "--old", "a", "--new", "b", "--occurrence", "2"],
            None
        )["occurrence"],
        2
    );
    // Text that spells no such value stays text, and the one refusal names
    // its field beside every other failure without repeating a value.
    let error = refusal(&filer, &["measure", "--v", "twelve", "--quiet=maybe"], None);
    assert!(error.starts_with("Invalid command arguments: "), "{error}");
    for failure in [
        "\"v.0\" is not of type \"number\"",
        "\"quiet\" is not of type \"boolean\"",
    ] {
        assert!(error.contains(failure), "{error} lacks {failure}");
    }
    assert!(
        !error.contains("twelve") && !error.contains("maybe"),
        "{error}"
    );
    let error = refusal(&filer, &["edit", "f", "--occurrence", "NaN"], None);
    for failure in [
        "\"occurrence\" is not of type \"integer\"",
        "\"old\" is a required property",
        "\"new\" is a required property",
    ] {
        assert!(error.contains(failure), "{error} lacks {failure}");
    }
    assert_eq!(
        refusal(&filer, &["create", "note.txt"], Some("a long body")),
        "Invalid command arguments: \"content\" is longer than 8 characters"
    );
}

#[test]
fn a_command_is_found_and_named_by_its_full_path() {
    // A bare root leaf reads its arguments right after its name.
    let kcenv: Node = serde_json::from_value(json!({"name": "kcenv",
        "summary": "Read an environment key.", "kind": "rpc",
        "input": object(json!({"key": {"type": "string"}}), &["key"]),
        "positionals": ["key"]}))
    .unwrap();
    assert_eq!(
        serde_json::to_value(read(&kcenv, &["HOME"], None).unwrap()).unwrap(),
        json!({"path": ["kcenv"], "help": false, "values": {"key": "HOME"}, "json": false})
    );
    let filer = filer();
    // A group with nothing after it asks for its help.
    let group = read(&filer, &["watch"], None).unwrap();
    assert!(group.help);
    assert_eq!(group.path, ["filer", "watch"]);
    assert_eq!(
        values(&filer, &["watch", "get", "my-id"], None),
        json!({"id": "my-id"})
    );
    assert_eq!(
        refusal(&filer, &["watch", "missing"], None),
        "Unknown subcommand \"filer watch missing\""
    );
    assert_eq!(
        refusal(&filer, &["watch", "get", "my-id", "--missing", "x"], None),
        "Unknown option \"--missing\" for \"filer watch get\""
    );
}

#[test]
fn json_output_is_offered_and_accepted_only_where_declared() {
    let filer = filer();
    let help = filer.help("filer");
    // Help shows an enum's choices, and --json only where an output schema is.
    assert!(
        help.contains("  filer status [--status <pending|in_progress|done>]\n"),
        "{help}"
    );
    assert!(help.contains("  filer list [--json]\n"), "{help}");
    assert!(read(&filer, &["list", "--json"], None).unwrap().json);
    assert_eq!(
        refusal(&filer, &["status", "--json"], None),
        "Command \"filer status\" does not define JSON output"
    );
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
