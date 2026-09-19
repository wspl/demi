//! No call is refused for how many others are in flight: past any count a new
//! call starts, and cancelling calls never ends the connection.
use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc, time::Duration};

use bytes::Bytes;
use demi_command_service::protocol::{Completion, Invocation, Record};
use demi_command_service::{
    Client, CommandInput, CommandOutput, Handler, InvocationContext, ServiceError, serve,
};
use tokio::task::JoinSet;

struct Fixture;

impl Handler for Fixture {
    fn operations(&self) -> Vec<String> {
        ["hold", "short", "flood"].map(String::from).to_vec()
    }

    fn invoke(
        &self,
        mut context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        Box::pin(async move {
            match context.request.operation.as_str() {
                // Held until the caller ends input, like a browser wait.
                "hold" => while context.input.next().await?.is_some() {},
                "flood" => loop {
                    context
                        .output
                        .stdout(Bytes::from(vec![7; 64 * 1024]))
                        .await?;
                },
                _ => context.output.stdout(Bytes::from_static(b"ok")).await?,
            }
            Ok(Completion {
                exit_code: 0,
                error: None,
            })
        })
    }
}

fn request(operation: &str) -> Invocation {
    Invocation {
        caller: "node".into(),
        conversation: "conversation".into(),
        json: None,
        edits: None,
        operation: operation.into(),
        invocation_id: operation.into(),
        args: serde_json::json!({}),
        cwd: "/tmp".into(),
        env: BTreeMap::new(),
    }
}

async fn connected() -> Client {
    let (client_io, server_io) = tokio::io::duplex(1024 * 1024);
    tokio::spawn(serve(server_io, Arc::new(Fixture)));
    let (client, connection) = Client::connect(client_io).await.unwrap();
    tokio::spawn(connection);
    client
}

async fn completed(mut output: CommandOutput) -> Result<(), String> {
    let mut code = None;
    while let Some(record) = output.next().await.map_err(|error| error.to_string())? {
        if let Record::Completion(completion) = record {
            code = Some(completion.exit_code);
        }
    }
    (code == Some(0))
        .then_some(())
        .ok_or_else(|| "no completion".into())
}

async fn short(client: &Client) -> Result<(), String> {
    let (_input, output) = client
        .invoke(&request("short"))
        .await
        .map_err(|error| error.to_string())?;
    completed(output).await
}

async fn hold(client: &Client) -> (CommandInput, CommandOutput) {
    client.invoke(&request("hold")).await.unwrap()
}

#[tokio::test(flavor = "multi_thread")]
async fn held_calls_beyond_any_count_all_start_and_finish() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let client = connected().await;
        let mut held = Vec::new();
        for _ in 0..256 {
            held.push(hold(&client).await);
        }
        let mut finishing = JoinSet::new();
        for (mut input, output) in held {
            finishing.spawn(async move {
                input.end().unwrap();
                completed(output).await
            });
        }
        while let Some(result) = finishing.join_next().await {
            result.unwrap().unwrap();
        }
        short(&client).await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn cancelling_a_call_never_turns_away_the_next() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let client = connected().await;
        let mut held = Vec::new();
        for _ in 0..64 {
            held.push(hold(&client).await);
        }
        for _ in 0..200 {
            let (mut input, _output) = held.remove(0);
            input.cancel();
            held.push(hold(&client).await);
        }
        short(&client).await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn unread_outputs_never_hold_back_an_independent_call() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let client = connected().await;
        let mut floods = Vec::new();
        for _ in 0..64 {
            let (_input, mut output) = client.invoke(&request("flood")).await.unwrap();
            assert!(matches!(
                output.next().await.unwrap(),
                Some(Record::Stdout(_))
            ));
            floods.push((_input, output));
        }
        // Each flood now fills its own window and nobody reads it.
        tokio::time::sleep(Duration::from_millis(100)).await;
        tokio::time::timeout(Duration::from_secs(1), short(&client))
            .await
            .expect("an independent call finishes beside unread outputs")
            .unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn abandoning_a_burst_of_calls_keeps_the_connection() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let client = connected().await;
        let stop = tokio_util::sync::CancellationToken::new();
        let mut calls = JoinSet::new();
        for _ in 0..1000 {
            let client = client.clone();
            let stop = stop.clone();
            calls.spawn(async move {
                let held = request("hold");
                tokio::select! {
                    result = client.invoke(&held) => {
                        if let Ok((mut input, _output)) = result {
                            input.cancel();
                        }
                    }
                    _ = stop.cancelled() => {}
                }
            });
        }
        tokio::time::sleep(Duration::from_millis(1)).await;
        stop.cancel();
        while calls.join_next().await.is_some() {}
        short(&client).await.unwrap();
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn many_callers_back_to_back_all_succeed() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let client = connected().await;
        let mut callers = JoinSet::new();
        for _ in 0..128 {
            let client = client.clone();
            callers.spawn(async move {
                for _ in 0..50 {
                    short(&client).await?;
                }
                Ok::<_, String>(())
            });
        }
        while let Some(result) = callers.join_next().await {
            result.unwrap().unwrap();
        }
    })
    .await
    .unwrap();
}
