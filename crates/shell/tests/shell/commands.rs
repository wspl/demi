use std::{cell::RefCell, collections::BTreeMap, rc::Rc};

use demi_command_tree::NativeOperation;
use demi_shell::{
    Call, CommandSet, GroupBuilder, LeafBuilder, RESERVED_NAMES, RpcError, RpcInvocation, RpcPort,
    TypedRpc,
    testing::{MemoryPort, MemoryStorage, test_command_context},
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

/// The input of `demi todo add`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddArgs {
    /// Todo text
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
    let text = call.args.text;
    let added = port
        .update("todos", |current: Option<Vec<String>>| {
            let mut items = current.unwrap_or_default();
            items.extend((0..copies).map(|_| text.clone()));
            Ok(items)
        })
        .await?;
    port.stdout(format!("{}\n", added.len())).await?;
    Ok(0)
}

fn todo() -> GroupBuilder {
    GroupBuilder::new("demi", "Demi commands.").group(
        GroupBuilder::new("todo", "Manage the todo list.")
            .leaf(
                LeafBuilder::rpc("add", "Add a new todo.")
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
                        package: "demi.builtin".into(),
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
    set.register(todo()).unwrap();
    assert!(
        set.register(todo())
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
                .positionals(["text"])
                .stdin_field("text")
                .bind(TypedRpc::new(add)),
        ),
    );
    assert!(error.contains("multiple input sources for text"), "{error}");
    let error = refused(
        GroupBuilder::new("demi", "Demi.").leaf(
            LeafBuilder::rpc("add", "Add.")
                .input::<AddArgs>()
                .positionals(["count", "text"])
                .bind(TypedRpc::new(add)),
        ),
    );
    assert!(
        error.contains("required positional follows optional positional"),
        "{error}"
    );
    let error = refused(
        GroupBuilder::new("demi", "Demi.").leaf(
            LeafBuilder::native(
                "read",
                "Read.",
                NativeOperation {
                    package: "demi.builtin".into(),
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

#[test]
fn help_opens_with_the_defaults_and_lists_every_root() {
    assert_eq!(CommandSet::new().render_help(), "");
    let mut set = CommandSet::new();
    set.register(todo()).unwrap();
    let help = set.render_help();
    assert!(help.starts_with(demi_command_tree::HELP_DEFAULTS), "{help}");
    let root = set.declarations().next().unwrap().help("demi");
    assert_eq!(
        help,
        format!("{}\n\n{root}", demi_command_tree::HELP_DEFAULTS)
    );
    assert!(
        root.contains("demi todo add <text> [--count <count>] [--json]"),
        "{root}"
    );
    assert!(root.contains("How many copies"), "{root}");
}

#[tokio::test(flavor = "current_thread")]
async fn dispatch_validates_wire_arguments_as_they_are_and_runs_the_handler() {
    let mut set = CommandSet::new();
    set.register(todo()).unwrap();
    let memory = MemoryPort::new();
    let cancel = CancellationToken::new();
    let call = |args: Value| {
        set.dispatch(
            invocation(&["demi", "todo", "add"], args),
            memory.port(cancel.clone()),
        )
    };
    // Wire arguments are decoded JSON: "7" for a number is not converted.
    let error = call(json!({"text": "a", "count": "7"})).await.unwrap_err();
    assert!(
        matches!(&error, RpcError::Usage(text) if text.starts_with("Invalid command arguments")),
        "{error}"
    );
    assert!(call(json!({"text": "a", "extra": 1})).await.is_err());
    // A length bound counts Unicode scalar values: one scalar value, two
    // UTF-16 units.
    assert_eq!(call(json!({"text": "𝄞", "count": 2})).await.unwrap(), 0);
    assert!(call(json!({"text": "ab"})).await.is_err());
    assert_eq!(memory.stdout(), b"2\n");
    let native = set.dispatch(
        invocation(&["demi", "todo", "read"], json!({"text": "a"})),
        memory.port(cancel.clone()),
    );
    assert!(
        matches!(native.await, Err(RpcError::Usage(text)) if text.contains("not an rpc command"))
    );
}

#[tokio::test(flavor = "current_thread")]
async fn concurrent_updates_keep_both_writes() {
    let storage = MemoryStorage::new();
    let first = MemoryPort::with_storage(storage.clone());
    let second = MemoryPort::with_storage(storage.clone());
    let append = |port: RpcPort, item: &'static str| async move {
        port.update("todos", |current: Option<Vec<String>>| {
            let mut items = current.unwrap_or_default();
            items.push(item.into());
            Ok(items)
        })
        .await
    };
    let cancel = CancellationToken::new();
    let (a, b) = tokio::join!(
        append(first.port(cancel.clone()), "A"),
        append(second.port(cancel), "B")
    );
    a.unwrap();
    b.unwrap();
    let stored = storage.value("todos").unwrap();
    let mut items: Vec<String> = serde_json::from_value(stored).unwrap();
    items.sort();
    assert_eq!(items, ["A", "B"]);
}

#[tokio::test(flavor = "current_thread")]
async fn a_stored_value_the_handler_cannot_read_is_refused_not_replaced() {
    let storage = MemoryStorage::new();
    let port = MemoryPort::with_storage(storage.clone()).port(CancellationToken::new());
    port.storage(demi_shell::StorageOp::WriteIf {
        key: "todos".into(),
        value: Some(json!({"not": "a list"})),
        expected: None,
    })
    .await
    .unwrap();
    let result = port
        .update("todos", |current: Option<Vec<String>>| {
            Ok(current.unwrap_or_default())
        })
        .await;
    assert!(matches!(result, Err(RpcError::Failed(text)) if text.contains("unreadable")));
    assert_eq!(storage.value("todos"), Some(json!({"not": "a list"})));
}

#[tokio::test(flavor = "current_thread")]
async fn grafting_replaces_or_appends_and_filtering_drops_emptied_groups() {
    let mut set = CommandSet::new();
    set.register(todo()).unwrap();
    let calls = Rc::new(RefCell::new(0));
    let counted = calls.clone();
    let agent =
        GroupBuilder::new("agent", "Agents.").leaf(LeafBuilder::rpc("list", "List agents.").bind(
            TypedRpc::new(move |_: Call<Map<String, Value>>, _: RpcPort| {
                *counted.borrow_mut() += 1;
                async { Ok(0) }
            }),
        ));
    set.graft(&["demi"], agent).unwrap();
    let names = |set: &CommandSet| match set.declarations().next().unwrap() {
        demi_command_tree::Node::Group(group) => group
            .subcommands
            .iter()
            .map(|child| child.name().to_owned())
            .collect::<Vec<_>>(),
        demi_command_tree::Node::Leaf(_) => panic!("demi is a group"),
    };
    assert_eq!(names(&set), ["todo", "agent"]);
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
        set.graft(&["demi", "todo", "add"], LeafBuilder::rpc("x", "X."))
            .is_err()
    );

    let narrowed = set.filter(|path| path.get(1).map(String::as_str) != Some("todo"));
    assert_eq!(names(&narrowed), ["agent"]);
    let nothing = set.filter(|_| false);
    assert_eq!(nothing.declarations().count(), 0);
}
