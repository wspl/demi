//! The `demi todo` group: a task list the node keeps in its command storage
//! under one key (`command-state-history.md`). Each handler reads the key, or
//! updates it by compare-and-set on the node's revision, so concurrent
//! commands keep every task and give each its own id; the agent runtime
//! versions the key with the conversation's history.

use demi_shell::{
    Call, GroupBuilder, LeafBuilder, RpcError, RpcPort, StorageOp, StorageReply, TypedRpc,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The command storage key of the list.
const STORAGE_KEY: &str = "todos.json";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum TodoStatus {
    Pending,
    InProgress,
    Done,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct TodoItem {
    id: String,
    text: String,
    status: TodoStatus,
}

/// The input of `demi todo add`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddArgs {
    /// Todo text
    text: String,
}

/// The input of `demi todo update`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct UpdateArgs {
    /// Todo id
    id: String,
    /// Replacement text
    text: Option<String>,
    /// Replacement status
    status: Option<TodoStatus>,
}

/// The input of `demi todo done`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DoneArgs {
    /// Todo id
    id: String,
}

/// What `demi todo list --json` prints.
#[derive(Serialize, JsonSchema)]
struct TodoList {
    todos: Vec<TodoItem>,
}

/// What the other commands print with `--json`: the todo they changed.
#[derive(Serialize, JsonSchema)]
struct OneTodo {
    todo: TodoItem,
}

pub(crate) fn todo_group() -> GroupBuilder {
    let changed =
        |name: &str, summary: &str, failure: &str| {
            LeafBuilder::rpc(name, summary)
            .positionals(["id"])
            .json_output::<OneTodo>()
            .success_output(format!(
                "writes the {} todo as raw text, or JSON matching {{ todo }} when --json is passed",
                if name == "done" { "completed" } else { "updated" }
            ))
            .failure_output(failure)
        };
    GroupBuilder::new("todo", "Manage an agent-session-scoped task list for coding work.")
        .leaf(
            LeafBuilder::rpc("list", "List todos for the current agent session.")
                .json_output::<TodoList>()
                .success_output(
                    "writes the session todo list as raw text, or JSON matching { todos } when --json is passed",
                )
                .failure_output("writes storage or validation errors to stderr and exits non-zero")
                .bind(TypedRpc::new(list)),
        )
        .leaf(
            LeafBuilder::rpc("add", "Add a new todo.")
                .input::<AddArgs>()
                .positionals(["text"])
                .json_output::<OneTodo>()
                .success_output(
                    "writes the created todo as raw text, or JSON matching { todo } when --json is passed",
                )
                .failure_output("writes validation or storage errors to stderr and exits non-zero")
                .bind(TypedRpc::new(add)),
        )
        .leaf(
            changed(
                "update",
                "Update todo text or status.",
                "writes \"Todo not found\" or validation/storage errors to stderr and exits non-zero",
            )
            .input::<UpdateArgs>()
            .bind(TypedRpc::new(update)),
        )
        .leaf(
            changed(
                "done",
                "Mark a todo as done.",
                "writes \"Todo not found\" or validation/storage errors to stderr and exits non-zero",
            )
            .input::<DoneArgs>()
            .bind(TypedRpc::new(done)),
        )
}

async fn list(call: Call<Map<String, Value>>, port: RpcPort) -> Result<u8, RpcError> {
    let StorageReply::Value { value, .. } = port
        .storage(StorageOp::Read {
            key: STORAGE_KEY.into(),
        })
        .await?
    else {
        return Err(RpcError::Failed(format!(
            "reading {STORAGE_KEY} answered no value"
        )));
    };
    let todos = match value {
        Some(value) => serde_json::from_value::<Vec<TodoItem>>(value).map_err(|error| {
            RpcError::Failed(format!("stored {STORAGE_KEY} is unreadable: {error}"))
        })?,
        None => Vec::new(),
    };
    let text = if call.invocation.json {
        json(&TodoList { todos })?
    } else if todos.is_empty() {
        "No todos.\n".to_owned()
    } else {
        todos
            .iter()
            .map(|todo| format!("{}\n", line(todo)))
            .collect()
    };
    port.stdout(text).await?;
    Ok(0)
}

async fn add(call: Call<AddArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let text = call.args.text;
    let todos = port
        .update(STORAGE_KEY, |current: Option<Vec<TodoItem>>| {
            let mut todos = current.unwrap_or_default();
            let id = next_id(&todos);
            todos.push(TodoItem {
                id,
                text: text.clone(),
                status: TodoStatus::Pending,
            });
            Ok(todos)
        })
        .await?;
    let todo = todos.last().expect("the update appended a todo").clone();
    write(&port, call.invocation.json, todo).await
}

async fn update(call: Call<UpdateArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let UpdateArgs { id, text, status } = call.args;
    let todo = change(&port, &id, |todo| {
        if let Some(text) = &text {
            todo.text = text.clone();
        }
        if let Some(status) = status {
            todo.status = status;
        }
    })
    .await?;
    write(&port, call.invocation.json, todo).await
}

async fn done(call: Call<DoneArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let todo = change(&port, &call.args.id, |todo| todo.status = TodoStatus::Done).await?;
    write(&port, call.invocation.json, todo).await
}

/// Changes the todo `id` by compare-and-set and returns it as committed.
async fn change(
    port: &RpcPort,
    id: &str,
    mut edit: impl FnMut(&mut TodoItem),
) -> Result<TodoItem, RpcError> {
    let todos = port
        .update(STORAGE_KEY, |current: Option<Vec<TodoItem>>| {
            let mut todos = current.unwrap_or_default();
            let todo = todos
                .iter_mut()
                .find(|todo| todo.id == id)
                .ok_or_else(|| RpcError::Failed(format!("Todo not found: {id}")))?;
            edit(todo);
            Ok(todos)
        })
        .await?;
    Ok(todos
        .into_iter()
        .find(|todo| todo.id == id)
        .expect("the committed list holds the changed todo"))
}

/// Prints one todo, as a line or as JSON.
async fn write(port: &RpcPort, as_json: bool, todo: TodoItem) -> Result<u8, RpcError> {
    let text = if as_json {
        json(&OneTodo { todo })?
    } else {
        format!("{}\n", line(&todo))
    };
    port.stdout(text).await?;
    Ok(0)
}

fn json(value: &impl Serialize) -> Result<String, RpcError> {
    serde_json::to_string(value).map_err(|error| RpcError::Failed(error.to_string()))
}

/// One more than the largest `T<n>` id.
fn next_id(todos: &[TodoItem]) -> String {
    let largest = todos
        .iter()
        .filter_map(|todo| todo.id.strip_prefix('T')?.parse::<u64>().ok())
        .max()
        .unwrap_or(0);
    format!("T{}", largest + 1)
}

/// A todo as a line: its status mark, id and text.
fn line(todo: &TodoItem) -> String {
    let mark = match todo.status {
        TodoStatus::Done => 'x',
        TodoStatus::InProgress => '-',
        TodoStatus::Pending => ' ',
    };
    format!("[{mark}] {} {}", todo.id, todo.text)
}

#[cfg(test)]
mod tests {
    use std::rc::Rc;

    use demi_command_tree::Node;
    use demi_shell::{
        CommandSet, RpcError, RpcInvocation,
        testing::{MemoryPort, MemoryStorage, test_command_context},
    };
    use serde_json::{Value, json};
    use tokio_util::sync::CancellationToken;

    use crate::command_line::{argv, demi, parse};

    /// Runs `demi <line>` over `storage`: what it printed, or why it failed.
    async fn call(
        demi: &(CommandSet, Node),
        storage: &Rc<MemoryStorage>,
        line: &[&str],
    ) -> Result<String, RpcError> {
        let (commands, root) = demi;
        let parsed = parse(root, line, None).unwrap();
        let port = MemoryPort::with_storage(storage.clone());
        let invocation = RpcInvocation {
            path: parsed.path,
            argv: argv(line),
            args: parsed.values,
            json: parsed.json,
            cwd: "/workspace".into(),
            env: Default::default(),
            context: test_command_context(),
            caller: None,
            stdin: false,
            pipes: None,
        };
        let code = commands
            .dispatch(invocation, port.port(CancellationToken::new()))
            .await?;
        assert_eq!(code, 0);
        Ok(String::from_utf8(port.stdout()).unwrap())
    }

    async fn run(demi: &(CommandSet, Node), storage: &Rc<MemoryStorage>, line: &[&str]) -> String {
        call(demi, storage, line).await.unwrap()
    }

    fn json(printed: &str) -> Value {
        serde_json::from_str(printed).unwrap()
    }

    #[tokio::test]
    async fn todos_are_added_listed_changed_and_done_as_lines_or_json() {
        let demi = demi();
        let storage = MemoryStorage::new();
        assert_eq!(run(&demi, &storage, &["todo", "list"]).await, "No todos.\n");
        assert_eq!(
            run(&demi, &storage, &["todo", "add", "Run tests"]).await,
            "[ ] T1 Run tests\n"
        );
        assert_eq!(
            json(&run(&demi, &storage, &["todo", "add", "Write docs", "--json"]).await),
            json!({"todo": {"id": "T2", "text": "Write docs", "status": "pending"}})
        );
        assert_eq!(
            run(&demi, &storage, &["todo", "list"]).await,
            "[ ] T1 Run tests\n[ ] T2 Write docs\n"
        );
        assert_eq!(
            run(
                &demi,
                &storage,
                &["todo", "update", "T1", "--text", "Run full tests"]
            )
            .await,
            "[ ] T1 Run full tests\n"
        );
        assert_eq!(
            json(
                &run(
                    &demi,
                    &storage,
                    &["todo", "update", "T1", "--status", "in_progress", "--json"]
                )
                .await
            ),
            json!({"todo": {"id": "T1", "text": "Run full tests", "status": "in_progress"}})
        );
        assert_eq!(
            run(&demi, &storage, &["todo", "list"]).await,
            "[-] T1 Run full tests\n[ ] T2 Write docs\n"
        );
        assert_eq!(
            run(&demi, &storage, &["todo", "done", "T2"]).await,
            "[x] T2 Write docs\n"
        );
        assert_eq!(
            json(&run(&demi, &storage, &["todo", "done", "T1", "--json"]).await),
            json!({"todo": {"id": "T1", "text": "Run full tests", "status": "done"}})
        );
        assert_eq!(
            json(&run(&demi, &storage, &["todo", "list", "--json"]).await),
            json!({"todos": [
                {"id": "T1", "text": "Run full tests", "status": "done"},
                {"id": "T2", "text": "Write docs", "status": "done"},
            ]})
        );
        // An unknown todo fails and changes nothing.
        let failed = call(&demi, &storage, &["todo", "done", "T9"])
            .await
            .unwrap_err();
        assert_eq!(failed.to_string(), "Todo not found: T9");
        assert_eq!(
            storage
                .value("todos.json")
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            2
        );
    }

    #[tokio::test]
    async fn concurrent_adds_keep_every_todo_with_its_own_id() {
        let demi = demi();
        let storage = MemoryStorage::new();
        // Each request of the in-memory port yields first, so both adds read
        // the empty list before either writes, and one retries on conflict.
        let (first, second) = tokio::join!(
            run(&demi, &storage, &["todo", "add", "first", "--json"]),
            run(&demi, &storage, &["todo", "add", "second", "--json"]),
        );
        let mut ids = [json(&first), json(&second)].map(|added| added["todo"]["id"].clone());
        ids.sort_by_key(|id| id.to_string());
        assert_eq!(ids, [json!("T1"), json!("T2")]);
        let listed = json(&run(&demi, &storage, &["todo", "list", "--json"]).await);
        let mut texts: Vec<&str> = listed["todos"]
            .as_array()
            .unwrap()
            .iter()
            .map(|todo| todo["text"].as_str().unwrap())
            .collect();
        texts.sort_unstable();
        assert_eq!(texts, ["first", "second"]);
    }
}
