//! Declarations, help and command lines: the recorded cases of
//! `fixtures/cli.json` (a declaration, its help and command lines with what
//! each fills), how a command line fills each field and is refused, and the
//! declaration rules' refusals.

use demi_command_declarations::{Node, Parsed};
use serde_json::{Value, json};

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/cli.json")).unwrap()
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

/// What `--help` prints for the command `argv` names below `tree`.
fn help(tree: &Node, argv: &[&str]) -> String {
    let argv: Vec<String> = argv.iter().map(|token| (*token).to_owned()).collect();
    let selected = tree.select(&argv).unwrap();
    selected.node.help(&selected.path.join(" "))
}

/// Why a command line is refused.
fn refusal(tree: &Node, argv: &[&str], stdin: Option<&str>) -> String {
    read(tree, argv, stdin)
        .err()
        .unwrap_or_else(|| panic!("{argv:?} is accepted"))
}

#[test]
fn help_and_command_lines_match_the_recorded_cases() {
    let fixture = fixture();
    let tree = fixture_tree();
    tree.validate().unwrap();
    // A group's help lists its operations; a command's gives its usage.
    assert_eq!(
        tree.help("fixture"),
        "fixture: CLI fixture.\n\nSubcommands:\n  fixture read — Read a native file."
    );
    assert_eq!(help(&tree, &["read"]), fixture["help"].as_str().unwrap());
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
                    "ref": {"type": "string", "pattern": "^e[1-9][0-9]*$"},
                    "exact": {"type": "boolean"}, "dy": {"type": "integer"}}), &["tab"]),
                "positionals": ["tab", "ref"], "positionalOptions": ["ref"]},
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

/// What `filer <line>` comes to on stderr when it is refused: clap's
/// error, Demi's tip, the command's usage and the pointer to `--help`.
fn usage_error(line: &[&str], stdin: Option<&str>) -> String {
    refusal(&filer(), line, stdin)
}

#[test]
fn a_usage_error_is_clap_s_with_the_command_s_usage_and_demi_s_tips() {
    let cases: &[(&[&str], Option<&str>, &str)] = &[
        // One value too many, as `demi file read a.png b.png` was.
        (
            &["edit", "a.txt", "b.txt", "--old", "a", "--new", "b"],
            None,
            "error: unexpected argument 'b.txt' found\n\nUsage: filer edit <path> --old <old> --new <new> [--occurrence <occurrence>]\n\nFor more information, try '--help'.",
        ),
        // A near name for a misspelt option and a misspelt command.
        (
            &["edit", "a.txt", "--olf", "a"],
            None,
            "error: unexpected argument '--olf' found\n\n  tip: a similar argument exists: '--old'\n\nUsage: filer edit <path> --old <old> --new <new> [--occurrence <occurrence>]\n\nFor more information, try '--help'.",
        ),
        (
            &["crete", "a.txt"],
            None,
            "error: unrecognized subcommand 'crete'\n\n  tip: a similar subcommand exists: 'create'\n\nUsage: filer <command>\n\nFor more information, try '--help'.",
        ),
        // A body read from stdin, given as an option.
        (
            &["create", "a.txt", "--content", "inline"],
            Some("body"),
            "error: unexpected argument '--content' found\n\n  tip: \"filer create\" reads content only from stdin; remove --content and use a quoted heredoc\n\nUsage: filer create <path>\n\nFor more information, try '--help'.",
        ),
        // A positional, and the rest field, given as options.
        (
            &["create", "--path", "a.txt"],
            Some("body"),
            "error: unexpected argument '--path' found\n\n  tip: \"filer create\" takes <path> as a positional argument; remove --path and give the value alone\n\nUsage: filer create <path>\n\nFor more information, try '--help'.",
        ),
        (
            &["forward", "--args", "x"],
            None,
            "error: unexpected argument '--args' found\n\n  tip: \"filer forward\" takes args after --; remove --args and write them after --\n\nUsage: filer forward -- <args>...\n\nFor more information, try '--help'.",
        ),
        // A missing option value never swallows the next option, and the
        // usage line is there although clap leaves it out of this error.
        (
            &["edit", "a.txt", "--old", "--new", "b"],
            None,
            "error: a value is required for '--old <old>' but none was supplied\n\nUsage: filer edit <path> --old <old> --new <new> [--occurrence <occurrence>]\n\nFor more information, try '--help'.",
        ),
        (
            &["edit", "a.txt", "--old", "a", "--old", "b", "--new", "c"],
            None,
            "error: the argument '--old <old>' cannot be used multiple times\n\nUsage: filer edit <path> --old <old> --new <new> [--occurrence <occurrence>]\n\nFor more information, try '--help'.",
        ),
        (
            &["measure", "--v", "twelve"],
            None,
            "error: invalid value 'twelve' for '--v <v>': invalid float literal\n\nUsage: filer measure --v <v> [--label <label>] [--quiet]\n\nFor more information, try '--help'.",
        ),
        (
            &["status", "--status", "finished"],
            None,
            "error: invalid value 'finished' for '--status <pending|in_progress|done>'\n  [possible values: pending, in_progress, done]\n\nUsage: filer status [--status <pending|in_progress|done>]\n\nFor more information, try '--help'.",
        ),
        (
            &["upload"],
            None,
            "error: the following required arguments were not provided:\n  <path>...\n\nUsage: filer upload <path>...\n\nFor more information, try '--help'.",
        ),
        (
            &["status", "--json"],
            None,
            "error: unexpected argument '--json' found\n\nUsage: filer status [--status <pending|in_progress|done>]\n\nFor more information, try '--help'.",
        ),
        // A true or false after a flag is refused rather than taken as the
        // optional positional after it.
        (
            &["click", "t1", "--exact", "false"],
            None,
            "error: unexpected argument 'false' found\n\n  tip: --exact alone is true; write --exact=false for false\n\nUsage: filer click <tab> [<ref>] [--exact] [--dy <dy>]\n\nFor more information, try '--help'.",
        ),
        (
            &["click", "t1", "e3", "--ref", "e4"],
            None,
            "error: the argument '[ref]' cannot be used with '--ref <ref>'\n\nUsage: filer click <tab> [<ref>] [--exact] [--dy <dy>]\n\nFor more information, try '--help'.",
        ),
        // What clap cannot express, the validator checks after it, in the
        // same shape and one failure at a time.
        (
            &["create", "a.txt"],
            Some("a long body"),
            "error: \"content\" is longer than 8 characters\n\nUsage: filer create <path>\n\nFor more information, try '--help'.",
        ),
        (
            &["click", "t1", "x3"],
            None,
            "error: \"ref\" does not match \"^e[1-9][0-9]*$\"\n\nUsage: filer click <tab> [<ref>] [--exact] [--dy <dy>]\n\nFor more information, try '--help'.",
        ),
    ];
    for (line, stdin, expected) in cases {
        assert_eq!(usage_error(line, *stdin), *expected, "{line:?}");
    }
}

#[test]
fn arguments_that_arrive_as_json_are_refused_in_the_same_shape() {
    let filer = filer();
    let Node::Group(group) = &filer else {
        unreachable!("filer is a group")
    };
    let edit = group.subcommands[1].leaf().unwrap();
    let arguments = |value: Value| value.as_object().unwrap().clone();
    assert_eq!(
        edit.check_arguments(
            "filer edit",
            &arguments(json!({"path": "a", "old": "b", "new": "c", "occurrence": "7"}))
        )
        .unwrap_err()
        .to_string(),
        "error: \"occurrence\" is not of type \"integer\"\n\nUsage: filer edit <path> --old <old> --new <new> [--occurrence <occurrence>]\n\nFor more information, try '--help'."
    );
    edit.check_arguments(
        "filer edit",
        &arguments(json!({"path": "a", "old": "b", "new": "c", "occurrence": 7})),
    )
    .unwrap();
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
    // Help shows each field in its source's form, never a body as an option.
    let help = [
        help(&filer, &["create"]),
        help(&filer, &["forward"]),
        help(&filer, &["upload"]),
        help(&filer, &["key"]),
    ]
    .join("\n\n");
    for shown in [
        "  filer create <path> <<'EOF'\n  <content>\n  EOF\n",
        "    Stdin body: content - File content",
        "  filer forward -- <args>...\n",
        "  filer upload <path>...\n",
        "      <path>... (required, repeatable)",
        "  filer key <tab> [<ref>] <key>\n",
        "      <ref> (optional, or --ref <ref>)",
    ] {
        assert!(help.contains(shown), "{shown} not in {help}");
    }
    for option in ["--path", "--content", "--args"] {
        assert!(!help.contains(option), "{option} in {help}");
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

#[test]
fn registration_takes_the_one_optional_positional_before_a_required_last_and_positional_options_of_positionals() {
    let leaf = |positionals: Value, required: &[&str], options: Value| {
        serde_json::from_value::<Node>(json!({"name": "key", "summary": "Key.",
            "kind": "rpc",
            "input": object(json!({"a": {"type": "string"}, "b": {"type": "string"},
                "c": {"type": "string"}, "d": {"type": "string"}}), required),
            "positionals": positionals, "positionalOptions": options}))
        .unwrap()
        .validate()
        .map_err(|error| error.to_string())
    };
    leaf(json!(["a", "b", "c"]), &["a", "c"], json!(["b"])).unwrap();
    for (positionals, required) in [
        (json!(["a", "b", "c"]), &["c"][..]),
        (json!(["a", "b", "c", "d"]), &["a", "d"]),
        (json!(["a", "b", "c"]), &["b"]),
    ] {
        assert_eq!(
            leaf(positionals.clone(), required, json!([])).unwrap_err(),
            "required positional follows optional positional",
            "{positionals} requiring {required:?}"
        );
    }
    assert_eq!(
        leaf(json!(["a"]), &[], json!(["d"])).unwrap_err(),
        "positionalOptions names d, which is no positional"
    );
}

#[test]
fn a_leaf_runs_one_way_and_names_valid_inputs() {
    let rpc = json!({"name": "add", "summary": "Add", "kind": "rpc"});
    assert!(serde_json::from_value::<Node>(rpc.clone()).is_ok());
    let mut bound = rpc.clone();
    bound["binding"] = json!({"package": "demi.file", "operation": "file.read",
        "descriptorHash": "a".repeat(64)});
    assert!(serde_json::from_value::<Node>(bound).is_err());
    let unbound = json!({"name": "read", "summary": "Read", "kind": "native"});
    assert!(serde_json::from_value::<Node>(unbound).is_err());

    let declaration = |input: Value| {
        let mut leaf = rpc.clone();
        leaf["input"] = input;
        serde_json::from_value::<Node>(leaf).unwrap().validate()
    };
    assert!(
        declaration(json!({"type": "object", "properties": {"text": {"type": "string"}}})).is_ok()
    );
    assert!(declaration(json!({"type": "array"})).is_err());
    assert!(
        declaration(json!({"type": "object", "properties": {"json": {"type": "boolean"}}}))
            .is_err()
    );
    assert!(
        declaration(json!({"type": "object", "properties": {"bad name": {"type": "string"}}}))
            .is_err()
    );
    // Only named options decide whether the stdin field is read.
    let reading = |stdin_read: Value| {
        let mut leaf = rpc.clone();
        leaf["input"] = json!({"type": "object", "properties": {"path": {"type": "string"},
            "body": {"type": "string"}, "old": {"type": "string"}}});
        leaf["positionals"] = json!(["path"]);
        leaf["stdinField"] = json!("body");
        leaf["stdinRead"] = stdin_read;
        serde_json::from_value::<Node>(leaf).unwrap().validate()
    };
    assert!(reading(json!({"unless": ["old"]})).is_ok());
    assert!(reading(json!({"with": ["old"]})).is_ok());
    assert!(reading(json!({"with": []})).is_err());
    for field in ["path", "body", "absent"] {
        assert!(reading(json!({"unless": [field]})).is_err(), "{field}");
    }
}

#[test]
fn groups_name_distinct_subcommands() {
    let leaf = json!({"name": "add", "summary": "Add", "kind": "rpc"});
    let group = |subcommands: Value| {
        serde_json::from_value::<Node>(json!({"name": "note", "summary": "Notes",
            "subcommands": subcommands}))
        .unwrap()
        .validate()
    };
    assert!(group(json!([leaf.clone()])).is_ok());
    assert!(group(json!([leaf.clone(), leaf])).is_err());
    assert!(group(json!([])).is_err());
}

fn input(schema: Value) -> Result<(), String> {
    let schema: demi_command_declarations::Schema = serde_json::from_value(schema).unwrap();
    demi_command_declarations::check_input_subset(&schema).map_err(|error| error.to_string())
}

#[test]
fn the_input_subset_takes_scalars_enums_and_arrays_and_refuses_the_rest() {
    input(json!({"title": "Args", "type": "object", "additionalProperties": false,
        "required": ["path"], "properties": {
            "path": {"type": "string", "minLength": 1, "description": "A file"},
            "count": {"type": "integer", "format": "uint32", "minimum": 0},
            "mode": {"type": "string", "enum": ["fast", "slow"]},
            "tags": {"type": "array", "items": {"type": "string", "pattern": "^[a-z]+$"}, "maxItems": 3},
            "force": {"type": "boolean"}}}))
    .unwrap();

    let refused = |field: Value| {
        input(json!({"type": "object", "additionalProperties": false,
            "properties": {"field": field}}))
        .unwrap_err()
    };
    for (field, reason) in [
        (
            json!({"type": "integer", "default": 2}),
            "carries a default",
        ),
        (json!({"type": ["string", "null"]}), "allows null"),
        (
            json!({"type": "string", "enum": ["a", null]}),
            "allows null",
        ),
        (
            json!({"type": "object", "properties": {"inner": {"type": "string"}}}),
            "nested object",
        ),
        (
            json!({"oneOf": [{"type": "string"}, {"type": "integer"}]}),
            "union",
        ),
        (json!({"$ref": "#"}), "refers to another schema"),
        (
            json!({"type": "array", "items": {"type": "array", "items": {"type": "string"}}}),
            "array of arrays",
        ),
        (
            json!({"type": "array", "items": {"type": "string"}, "uniqueItems": true}),
            "\"uniqueItems\"",
        ),
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

#[test]
fn a_leaf_names_a_permission_category_one_of_its_groups_declares_and_its_help_says_so() {
    let manage = json!({"id": "skills.manage", "action": "manage skills",
        "description": "Add and remove skill sources."});
    let tree = |permissions: Value, leaf: Value| {
        serde_json::from_value::<Node>(json!({"name": "demi", "summary": "Demi",
            "subcommands": [{"name": "skills", "summary": "Skills",
                "permissions": permissions, "subcommands": [leaf]}]}))
        .unwrap()
    };
    let add = json!({"name": "add", "summary": "Add", "kind": "rpc",
        "permission": "skills.manage"});
    let declared = tree(json!([manage.clone()]), add.clone());
    declared.validate().unwrap();
    let help = help(&declared, &["skills", "add"]);
    assert!(
        help.contains("    Permission: needs the user's permission (skills.manage) in each conversation; without it, the command fails at once and the user is asked."),
        "{help}"
    );

    let refused = |tree: Node| tree.validate().unwrap_err().to_string();
    let undeclared = refused(tree(json!([]), add.clone()));
    assert!(undeclared.contains("none of its groups declares"), "{undeclared}");
    let native = json!({"name": "read", "summary": "Read", "kind": "native",
        "permission": "skills.manage",
        "binding": {"package": "demi.file", "operation": "file.read", "descriptorHash": "a".repeat(64)}});
    let on_native = refused(tree(json!([manage.clone()]), native));
    assert!(on_native.contains("only an rpc command can"), "{on_native}");
    let twice = refused(tree(json!([manage.clone(), manage.clone()]), add.clone()));
    assert!(twice.contains("declared twice"), "{twice}");
    let misnamed = json!({"id": "other.manage", "action": "manage", "description": "Manage."});
    let foreign = refused(tree(json!([misnamed]), add));
    assert!(foreign.contains("must be named skills.<name>"), "{foreign}");
}

#[test]
fn a_brings_host_field_is_a_string_field_of_an_rpc_leafs_input() {
    let tree = |leaf: Value| {
        serde_json::from_value::<Node>(json!({"name": "demi", "summary": "Demi",
            "subcommands": [{"name": "conversation", "summary": "Conversation",
                "subcommands": [leaf]}]}))
        .unwrap()
    };
    let input = json!({"type": "object", "additionalProperties": false,
        "properties": {"project": {"type": "string"}, "out": {"type": "boolean"}}});
    let leaf = |kind: &str, field: &str| {
        let mut leaf = json!({"name": "move", "summary": "Move", "kind": kind,
            "input": input.clone(), "bringsHost": field});
        if kind == "native" {
            leaf["binding"] = json!({"package": "demi.file", "operation": "file.read",
                "descriptorHash": "a".repeat(64)});
        }
        leaf
    };
    tree(leaf("rpc", "project")).validate().unwrap();
    let refused = |tree: Node| tree.validate().unwrap_err().to_string();
    let flag = refused(tree(leaf("rpc", "out")));
    assert!(flag.contains("no string field"), "{flag}");
    let missing = refused(tree(leaf("rpc", "device")));
    assert!(missing.contains("no string field"), "{missing}");
    let native = refused(tree(leaf("native", "project")));
    assert!(native.contains("only an rpc command can"), "{native}");
}
