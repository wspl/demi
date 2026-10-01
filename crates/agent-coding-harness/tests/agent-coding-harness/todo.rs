//! `demi todo`: the rpc group over a node's command storage.

use std::rc::Rc;

use demi_command_declarations::Node;
use demi_host_interface::{
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
