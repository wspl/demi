//! No command the backend implements is refused for how many others are in
//! flight (`runner.md` § Load).

use demi_command_protocol::{CommandCaller, CommandContext, CommandLocale, LocalInvocation};
use demi_runner_jobs::testing::Dispatch;
use demi_runner_process::{
    command_client::{RawCommand, Stdio, forward},
    pipes::PipeClient,
};
use serde_json::json;
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn command_context(conversation: &str) -> CommandContext {
    CommandContext {
        conversation: conversation.into(),
        caller: CommandCaller::agent(1),
        locale: CommandLocale {
            time_zone: "UTC".into(),
            languages: vec!["en-US".into()],
        },
    }
}

/// Backend-implemented commands are relayed however many are in flight.
#[tokio::test(flavor = "multi_thread")]
async fn backend_commands_are_never_turned_away() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let directory = tempfile::tempdir().unwrap();
        let cwd = directory.path().to_owned();
        let tree = json!({
            "name": "fixture", "summary": "Test callback.", "kind": "rpc", "runningHint": "Working",
            "input": {"type": "object", "properties": {"body": {"type": "string"}}, "required": ["body"]}, "stdinField": "body"
        });
        let manifest = demi_runner_protocol::manifest::Manifest::build(
            [serde_json::from_value(tree).unwrap()],
            [],
        )
        .unwrap();
        let manifest = serde_json::to_value(manifest).unwrap();
        let pipes = PipeClient::new(&"http://127.0.0.1:1".parse().unwrap(), tokio::sync::watch::Sender::new(None).subscribe()).unwrap();
        let mut dispatch = Dispatch::new(&cwd, manifest, pipes).await;
        let mut outgoing = std::mem::replace(&mut dispatch.outgoing, mpsc::channel(1).1);
        // The backend never answers; count the calls that reach it.
        let reached = Arc::new(AtomicUsize::new(0));
        let counted = reached.clone();
        tokio::spawn(async move {
            while let Some(message) = outgoing.recv().await {
                let value: serde_json::Value =
                    rmp_serde::from_slice(&message.into_bytes()).unwrap();
                if value["type"] == "rpc_call" {
                    counted.fetch_add(1, Ordering::SeqCst);
                }
            }
        });
        let (context, _guard) = dispatch.context("job", command_context("conversation")).await;
        let request = LocalInvocation {
            operation: "raw".into(),
            invocation_id: "raw".into(),
            args: serde_json::to_value(RawCommand {
                context: context.id.clone(),
                root: "fixture".into(),
                argv: vec![],
                live: true,
                stdout: demi_command_protocol::StdoutTarget::Job,
            })
            .unwrap(),
            cwd: cwd.to_string_lossy().into_owned(),
            env: BTreeMap::new(),
        };
        let cancel = CancellationToken::new();
        let mut running = tokio::task::JoinSet::new();
        for _ in 0..200 {
            let endpoint = dispatch.server.endpoint().to_owned();
            let request = request.clone();
            let cancel = cancel.clone();
            running.spawn(async move {
                let mut stderr = Vec::new();
                let result = forward(
                    &endpoint,
                    &request,
                    Stdio {
                        stdin: tokio::io::empty(),
                        stdout: tokio::io::sink(),
                        stderr: &mut stderr,
                    },
                    cancel,
                )
                .await;
                (result.map(|completion| completion.exit_code).map_err(|error| error.to_string()), stderr)
            });
        }
        while reached.load(Ordering::SeqCst) < 200 {
            if let Some(result) = running.try_join_next() {
                let (exit, stderr) = result.unwrap();
                panic!(
                    "a call ended while the backend held it: {exit:?} {}",
                    String::from_utf8_lossy(&stderr)
                );
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        cancel.cancel();
        while running.join_next().await.is_some() {}
        dispatch.close().await;
    })
    .await
    .unwrap();
}
