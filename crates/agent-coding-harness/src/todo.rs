//! The `demi todo` group: a task list the node keeps in its command storage
//! under one key (`command-state-history.md`). Each handler reads the key, or
//! updates it by compare-and-set on the node's revision, so concurrent
//! commands keep every task and give each its own id; the agent runtime
//! versions the key with the conversation's history.

use demi_host_interface::{
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
