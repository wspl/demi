//! How a command line fills each field of a declaration's input, and the
//! schemas a declared argument type generates.

use demi_command_declarations::{Node, Parsed};
use serde_json::{Value, json};

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

/// A leaf's input: an object with `properties` that requires `required` and
/// allows nothing else.
fn object(properties: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": properties, "required": required,
        "additionalProperties": false})
}

/// Commands with fields from every source.
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
            {"name": "upload", "summary": "Upload files.", "kind": "rpc",
                "input": object(json!({"path": {"type": "array", "items": {"type": "string"},
                    "minItems": 1}}), &["path"]),
                "positionals": ["path"]},
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
            {"name": "click", "summary": "Click an element.", "kind": "rpc",
                "input": object(json!({"tab": {"type": "string"},
                    "ref": {"type": "string", "pattern": "^e[1-9][0-9]*$",
                        "patternDescription": "a ref, such as e12"},
                    "exact": {"type": "boolean"}, "dy": {"type": "integer"}}), &["tab"]),
                "positionals": ["tab", "ref"], "positionalOptions": ["ref"]},
            {"name": "eval", "summary": "Evaluate an expression.", "kind": "rpc",
                "input": object(json!({"tab": {"type": "string"},
                    "ref": {"type": "string", "pattern": "^e[1-9][0-9]*$",
                        "patternDescription": "a ref, such as e12"},
                    "expression": {"type": "string"}}), &["tab", "expression"]),
                "positionals": ["tab", "ref", "expression"], "positionalOptions": ["ref"],
                "stdinField": "expression"},
            {"name": "key", "summary": "Press a key.", "kind": "rpc",
                "input": object(json!({"tab": {"type": "string"}, "ref": {"type": "string"},
                    "key": {"type": "string"}}), &["tab", "key"]),
                "positionals": ["tab", "ref", "key"], "positionalOptions": ["ref"]},
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
    // A standalone -- ends the options.
    assert_eq!(
        values(&filer, &["create", "--", "--help"], Some("body"))["path"],
        "--help"
    );
    // A rest field takes the tokens after -- as they are, and only those.
    assert_eq!(
        values(&filer, &["forward", "--", "--help", "--json"], None),
        json!({"args": ["--help", "--json"]})
    );
    // A trailing array positional takes every positional argument left.
    assert_eq!(
        values(&filer, &["upload", "out/login.png", "demo.mp4"], None),
        json!({"path": ["out/login.png", "demo.mp4"]})
    );
    // A positional with its option form takes either, never both.
    for line in [&["click", "t1", "e3"][..], &["click", "t1", "--ref", "e3"]] {
        assert_eq!(values(&filer, line, None), json!({"tab": "t1", "ref": "e3"}));
    }
    // An optional positional before a required last one: two tokens fill
    // the first and the last, three fill all three.
    assert_eq!(
        values(&filer, &["key", "t1", "Enter"], None),
        json!({"tab": "t1", "key": "Enter"})
    );
    assert_eq!(
        values(&filer, &["key", "t1", "e1", "Enter"], None),
        json!({"tab": "t1", "ref": "e1", "key": "Enter"})
    );
    // A stdin field may also be the last positional: the token after the
    // tab is the ref when it reads as one and otherwise the expression, and
    // given there, stdin is not read (the dispatcher passes no body).
    for (line, stdin, expected) in [
        (&["eval", "t1", "document.title"][..], None, json!({"tab": "t1", "expression": "document.title"})),
        (&["eval", "t1", "e21"], Some("element.value"), json!({"tab": "t1", "ref": "e21", "expression": "element.value"})),
        (&["eval", "t1", "e21", "element.id"], None, json!({"tab": "t1", "ref": "e21", "expression": "element.id"})),
        (&["eval", "t1", "--ref", "e21", "element.id"], None, json!({"tab": "t1", "ref": "e21", "expression": "element.id"})),
        (&["eval", "t1"], Some("document.title"), json!({"tab": "t1", "expression": "document.title"})),
    ] {
        assert_eq!(values(&filer, line, stdin), expected, "{line:?}");
    }
}

#[test]
fn a_value_converts_to_its_field_s_type_and_may_begin_with_a_single_dash() {
    let filer = filer();
    // Each element of a repeated option converts on its own.
    assert_eq!(
        values(&filer, &["measure", "--v", "12", "--v", "1.5", "--label", "a", "--quiet"], None),
        json!({"v": [12, 1.5], "label": ["a"], "quiet": true})
    );
    assert_eq!(
        values(&filer, &["measure", "--v", "1", "--quiet=false"], None)["quiet"],
        false
    );
    // A value beginning with one dash follows its option; one beginning with
    // -- is given as --name=value, as is an empty one.
    assert_eq!(
        values(&filer, &["click", "t1", "--dy", "-300", "--exact"], None),
        json!({"tab": "t1", "dy": -300, "exact": true})
    );
    assert_eq!(
        values(&filer, &["edit", "note.txt", "--old", "-x", "--new="], None),
        json!({"path": "note.txt", "old": "-x", "new": ""})
    );
    assert_eq!(
        values(&filer, &["edit", "note.txt", "--old=--help", "--new", "b"], None)["old"],
        "--help"
    );
}

#[test]
fn help_is_asked_anywhere_and_a_group_alone_asks_for_it() {
    let filer = filer();
    for line in [
        &["watch"][..],
        &["watch", "--help"],
        &["edit", "a.txt", "b.txt", "--bogus", "--help"],
        &["watch", "missing", "--help"],
    ] {
        assert!(read(&filer, line, None).unwrap().help, "{line:?}");
    }
    assert_eq!(read(&filer, &["watch", "missing", "--help"], None).unwrap().path, ["filer", "watch"]);
    // After --, --help is data.
    assert!(!read(&filer, &["create", "--", "--help"], Some("b")).unwrap().help);
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
    assert!(read(&filer, &["list", "--json"], None).unwrap().json);
    assert_eq!(
        values(&filer, &["watch", "get", "my-id"], None),
        json!({"id": "my-id"})
    );
}

fn input(schema: Value) -> Result<(), String> {
    let schema: demi_command_declarations::Schema = serde_json::from_value(schema).unwrap();
    demi_command_declarations::check_input_subset(&schema).map_err(|error| error.to_string())
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
    let schema = demi_command_declarations::command_schema_settings()
        .into_generator()
        .into_root_schema_for::<ExampleArgs>();
    let value = schema.to_value();
    assert!(value.get("$schema").is_none());
    assert!(!value.to_string().contains("null"), "{value}");
    assert_eq!(value["required"], json!(["path", "tags"]));
    assert_eq!(value["properties"]["count"]["maximum"], json!(9));
    assert_eq!(value["properties"]["mode"]["enum"], json!(["fast", "slow"]));
    input(value).unwrap();
}

