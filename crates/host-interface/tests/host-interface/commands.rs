use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use demi_command_declarations::NativeOperation;
use demi_host_interface::{
    Call, CommandSet, GroupBuilder, LeafBuilder, RESERVED_NAMES, RpcError, RpcInvocation, RpcPort,
    TypedRpc,
    testing::{MemoryPort, test_command_context},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

/// The input of `demi note add`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddArgs {
    /// Note text
    #[schemars(length(max = 1))]
    text: String,
    /// How many copies
    count: Option<u32>,
}

#[derive(Serialize, Deserialize, JsonSchema)]
struct Reply {
    added: Vec<String>,
}

async fn add(call: Call<AddArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let copies = call.args.count.unwrap_or(1);
    let text = call.args.text.repeat(copies as usize);
    port.stdout(format!("{text}\n")).await?;
    Ok(0)
}

fn notes() -> GroupBuilder {
    GroupBuilder::new("demi", "Demi commands.").group(
        GroupBuilder::new("note", "Manage notes.")
            .index_entry("Keeps notes.")
            .leaf(
                LeafBuilder::rpc("add", "Add a note.")
                    .input::<AddArgs>()
                    .positionals(["text"])
                    .json_output::<Reply>()
                    .bind(TypedRpc::new(add)),
            )
            .leaf(
                LeafBuilder::native(
                    "read",
                    "Read a file.",
                    NativeOperation {
                        package: "demi.file".into(),
                        operation: "file.read".into(),
                    },
                )
                .input::<AddArgs>(),
            ),
    )
}

fn invocation(path: &[&str], args: Value) -> RpcInvocation {
    let Value::Object(args) = args else {
        panic!("arguments are an object")
    };
    RpcInvocation {
        path: path.iter().map(|part| (*part).to_owned()).collect(),
        argv: Vec::new(),
        args,
        json: false,
        host: "laptop".into(),
        cwd: "/".into(),
        env: BTreeMap::new(),
        context: test_command_context(),
        caller: None,
        stdin: false,
        pipes: None,
    }
}

#[test]
fn registration_refuses_reserved_taken_malformed_and_unbound_commands() {
    let mut set = CommandSet::new();
    set.register(notes()).unwrap();
    assert!(
        set.register(notes())
            .unwrap_err()
            .to_string()
            .contains("already registered")
    );
    for name in ["git", "cd", "grep", "python3", "."] {
        assert!(RESERVED_NAMES.contains(&name));
        let error = CommandSet::new()
            .register(LeafBuilder::rpc(name, "Shadows a tool."))
            .unwrap_err();
        assert!(error.to_string().contains("reserved"), "{error}");
    }
    let refused = |root: GroupBuilder| CommandSet::new().register(root).unwrap_err().to_string();
    let error = refused(GroupBuilder::new("demi", "Demi.").leaf(LeafBuilder::rpc("add", "Add.")));
    assert!(error.contains("\"demi add\" has no handler"), "{error}");
    let error =
        refused(GroupBuilder::new("demi", "Demi.").group(GroupBuilder::new("empty", "Empty.")));
    assert!(error.contains("no subcommands"), "{error}");
    let error = refused(
        GroupBuilder::new("demi", "Demi.")
            .leaf(LeafBuilder::rpc("bad name", "Bad.").bind(TypedRpc::new(add))),
    );
    assert!(error.contains("invalid command name"), "{error}");
    let error = refused(
        GroupBuilder::new("demi", "Demi.").leaf(
            LeafBuilder::rpc("add", "Add.")
                .input::<AddArgs>()
                .positionals(["missing"])
                .bind(TypedRpc::new(add)),
        ),
    );
    assert!(error.contains("missing"), "{error}");
    let error = refused(
        GroupBuilder::new("demi", "Demi.").leaf(
            LeafBuilder::rpc("add", "Add.")
                .input::<AddArgs>()
                .stdin_field("count")
                .bind(TypedRpc::new(add)),
        ),
    );
    assert!(error.contains("stdin input must be a string"), "{error}");
    let error = refused(
        GroupBuilder::new("demi", "Demi.").leaf(
            LeafBuilder::rpc("add", "Add.")
                .input::<AddArgs>()
                // The stdin field may be a positional only as the last one.
                .positionals(["text", "count"])
                .stdin_field("text")
                .bind(TypedRpc::new(add)),
        ),
    );
    assert!(error.contains("multiple input sources for text"), "{error}");
    // One optional positional directly before a required last one is the
    // one such shape registration takes, as `key <tab> [<ref>] <key>`; the
    // command-tree library's tests hold the refused ones.
    CommandSet::new()
        .register(
            GroupBuilder::new("demi", "Demi.").leaf(
                LeafBuilder::rpc("add", "Add.")
                    .input::<AddArgs>()
                    .positionals(["count", "text"])
                    .bind(TypedRpc::new(add)),
            ),
        )
        .unwrap();
    let error = refused(
        GroupBuilder::new("demi", "Demi.").leaf(
            LeafBuilder::native(
                "read",
                "Read.",
                NativeOperation {
                    package: "demi.file".into(),
                    operation: "file.read".into(),
                },
            )
            .bind(TypedRpc::new(add)),
        ),
    );
    assert!(error.contains("takes no handler"), "{error}");
    let error = refused(
        GroupBuilder::new("demi", "Demi.").leaf(
            LeafBuilder::rpc("add", "Add.")
                .input::<AddArgs>()
                .describe("absent", "x")
                .bind(TypedRpc::new(add)),
        ),
    );
    assert!(error.contains("absent"), "{error}");
}

/// An input the subset refuses: a default and a nested object.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, reason = "read only through its schema")]
struct Defaulted {
    #[serde(default = "two")]
    count: u32,
}

fn two() -> u32 {
    2
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[allow(dead_code, reason = "read only through its schema")]
struct Nested {
    inner: AddArgs,
}

#[test]
fn registration_refuses_inputs_outside_the_subset_naming_the_field() {
    for (leaf, field, reason) in [
        (
            LeafBuilder::rpc("add", "Add.").input::<Defaulted>(),
            "count",
            "default",
        ),
        (
            LeafBuilder::rpc("add", "Add.").input::<Nested>(),
            "inner",
            "nested object",
        ),
    ] {
        let root = GroupBuilder::new("demi", "Demi.").leaf(leaf.bind(TypedRpc::new(add)));
        let error = CommandSet::new().register(root).unwrap_err().to_string();
        assert!(error.contains("\"demi add\""), "{error}");
        assert!(error.contains(&format!("input \"{field}\"")), "{error}");
        assert!(error.contains(reason), "{error}");
    }
}

/// A leaf that reads a file, for groups whose leaves no test runs.
fn read(name: &str) -> LeafBuilder {
    LeafBuilder::native(
        name,
        "Read a file.",
        NativeOperation {
            package: "demi.file".into(),
            operation: "file.read".into(),
        },
    )
}

#[test]
fn the_index_opens_with_the_defaults_and_lists_each_group_with_an_entry_by_path() {
    assert_eq!(CommandSet::new().render_index(), "");
    // Registered out of order: `zeta` under `demi`, then a root of its own
    // between them, then `alpha` grafted after. `demi` and `content`, which
    // declare no entry, are reached through the groups that list them.
    let mut set = CommandSet::new();
    set.register(
        GroupBuilder::new("demi", "Demi commands.").group(
            GroupBuilder::new("zeta", "Zeta.")
                .index_entry("Works the last letter.")
                .leaf(read("open"))
                .group(GroupBuilder::new("content", "Content.").leaf(read("fetch"))),
        ),
    )
    .unwrap();
    set.register(
        GroupBuilder::new("lint", "Lint.")
            .index_entry("Checks the code.")
            .leaf(read("run")),
    )
    .unwrap();
    set.graft(
        &["demi"],
        GroupBuilder::new("alpha", "Alpha.")
            .index_entry("Works the first letter.")
            .leaf(read("one"))
            .leaf(read("two")),
    )
    .unwrap();

    let index = set.render_index();

    let defaults = demi_command_declarations::HELP_DEFAULTS;
    let opener = demi_command_declarations::INDEX_OPENER;
    assert_eq!(
        index,
        format!(
            "{defaults}\n\n{opener}\n\n\
             demi alpha\nWorks the first letter.\nOperations: one, two\n\
             Details: demi alpha --help; one operation: demi alpha <operation> --help\n\n\
             demi zeta\nWorks the last letter.\nOperations: open, content fetch\n\
             Details: demi zeta --help; one operation: demi zeta <operation> --help\n\n\
             lint\nChecks the code.\nOperations: run\n\
             Details: lint --help; one operation: lint <operation> --help"
        )
    );
}

#[test]
fn registration_refuses_a_top_level_group_without_an_index_entry_or_with_one_too_long() {
    let notes = |entry: Option<String>| {
        let group = GroupBuilder::new("notes", "Notes.")
            .leaf(read("read"))
            // A subgroup is reached through its group and needs no entry.
            .group(GroupBuilder::new("content", "Content.").leaf(read("fetch")));
        let group = match entry {
            Some(entry) => group.index_entry(entry),
            None => group,
        };
        GroupBuilder::new("demi", "Demi commands.").group(group)
    };
    // Characters, not bytes: 600 of a two-byte letter fit.
    CommandSet::new()
        .register(notes(Some("é".repeat(600))))
        .unwrap();
    let too_long = CommandSet::new()
        .register(notes(Some("é".repeat(601))))
        .unwrap_err()
        .to_string();
    assert!(too_long.contains("notes"), "{too_long}");
    assert!(too_long.contains("601 characters"), "{too_long}");
    let missing = CommandSet::new().register(notes(None)).unwrap_err().to_string();
    assert!(missing.contains("\"demi notes\""), "{missing}");
    assert!(missing.contains("needs an index entry"), "{missing}");
    // A root of its own is a top-level group too, also when grafted onto.
    let mut set = CommandSet::new();
    let lint = set
        .register(GroupBuilder::new("lint", "Lint.").leaf(read("run")))
        .unwrap_err()
        .to_string();
    assert!(lint.contains("\"lint\": a top-level command group needs an index entry"), "{lint}");
    set.register(notes(Some("Keeps notes.".into()))).unwrap();
    let grafted = set
        .graft(&["demi"], GroupBuilder::new("todo", "Todo.").leaf(read("list")))
        .unwrap_err()
        .to_string();
    assert!(grafted.contains("\"demi todo\""), "{grafted}");
}

#[tokio::test(flavor = "current_thread")]
async fn dispatch_validates_wire_arguments_as_they_are_and_runs_the_handler() {
    let mut set = CommandSet::new();
    set.register(notes()).unwrap();
    let memory = MemoryPort::new();
    let cancel = CancellationToken::new();
    let call = |args: Value| {
        set.dispatch(
            invocation(&["demi", "note", "add"], args),
            memory.port(cancel.clone()),
        )
    };
    // Wire arguments are decoded JSON: "7" for a number is not converted.
    let error = call(json!({"text": "a", "count": "7"})).await.unwrap_err();
    assert_eq!(
        error,
        RpcError::Usage(
            "error: \"count\" is not of type \"integer\"\nUsage: demi note add <text> [--count <count>] [--json]; more with --help".into()
        )
    );
    assert!(call(json!({"text": "a", "extra": 1})).await.is_err());
    // A length bound counts Unicode scalar values: one scalar value, two
    // UTF-16 units.
    assert_eq!(call(json!({"text": "𝄞", "count": 2})).await.unwrap(), 0);
    assert!(call(json!({"text": "ab"})).await.is_err());
    assert_eq!(memory.stdout(), "𝄞𝄞\n".as_bytes());
    let native = set.dispatch(
        invocation(&["demi", "note", "read"], json!({"text": "a"})),
        memory.port(cancel.clone()),
    );
    assert!(
        matches!(native.await, Err(RpcError::Failed(text)) if text.contains("not an rpc command"))
    );
}

#[tokio::test(flavor = "current_thread")]
async fn grafting_replaces_or_appends_and_filtering_drops_emptied_groups() {
    let mut set = CommandSet::new();
    set.register(notes()).unwrap();
    let calls = Rc::new(RefCell::new(0));
    let counted = calls.clone();
    let agent =
        GroupBuilder::new("agent", "Agents.").index_entry("Runs agents.").leaf(LeafBuilder::rpc("list", "List agents.").bind(
            TypedRpc::new(move |_: Call<Map<String, Value>>, _: RpcPort| {
                *counted.borrow_mut() += 1;
                async { Ok(0) }
            }),
        ));
    set.graft(&["demi"], agent).unwrap();
    let names = |set: &CommandSet| match set.declarations().next().unwrap() {
        demi_command_declarations::Node::Group(group) => group
            .subcommands
            .iter()
            .map(|child| child.name().to_owned())
            .collect::<Vec<_>>(),
        demi_command_declarations::Node::Leaf(_) => panic!("demi is a group"),
    };
    assert_eq!(names(&set), ["note", "agent"]);
    let port = MemoryPort::new().port(CancellationToken::new());
    set.dispatch(
        invocation(&["demi", "agent", "list"], json!({})),
        port.clone(),
    )
    .await
    .unwrap();
    assert_eq!(*calls.borrow(), 1);
    // A graft of an unbound leaf is refused and leaves the set as it was.
    assert!(
        set.graft(&["demi"], LeafBuilder::rpc("agent", "Unbound."))
            .is_err()
    );
    set.dispatch(invocation(&["demi", "agent", "list"], json!({})), port)
        .await
        .unwrap();
    assert!(
        set.graft(&["demi", "note", "add"], LeafBuilder::rpc("x", "X."))
            .is_err()
    );

    let narrowed = set.filter(|path| path.get(1).map(String::as_str) != Some("note"));
    assert_eq!(names(&narrowed), ["agent"]);
    let nothing = set.filter(|_| false);
    assert_eq!(nothing.declarations().count(), 0);
}

#[test]
fn a_permission_category_is_declared_once_in_the_set_and_its_leaves_check_answers_it() {
    let skills = |root: &str| {
        GroupBuilder::new(root, "Roots.").index_entry("Holds roots.").group(
            GroupBuilder::new("skills", "Skills.")
                .index_entry("Manages skills.")
                .permission("skills.manage", "manage skills", "Add and remove sources.")
                .leaf(
                    LeafBuilder::rpc("add", "Add a note.")
                        .input::<AddArgs>()
                        .positionals(["text"])
                        .permission("skills.manage")
                        .bind(TypedRpc::new(add)),
                ),
        )
    };
    let mut set = CommandSet::new();
    set.register(skills("demi")).unwrap();
    let error = set.register(skills("other")).unwrap_err().to_string();
    assert!(error.contains("declared twice"), "{error}");

    let checked = set
        .check(&invocation(&["demi", "skills", "add"], json!({"text": "a"})))
        .unwrap();
    assert_eq!(
        checked.category.map(|category| category.id.as_str()),
        Some("skills.manage")
    );
    let unchecked = notes();
    let mut plain = CommandSet::new();
    plain.register(unchecked).unwrap();
    let checked = plain
        .check(&invocation(&["demi", "note", "add"], json!({"text": "a"})))
        .unwrap();
    assert!(checked.category.is_none());
}
