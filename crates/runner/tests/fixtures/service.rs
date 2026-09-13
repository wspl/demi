//! Deliberately faulty operations available only in native integration tests.
use bytes::Bytes;
use demi_command_service::protocol::Completion;
use demi_command_service::{Handler, InvocationContext, ServiceError};
use std::{future::Future, pin::Pin, sync::Arc};

struct Fixture;

impl Handler for Fixture {
    fn operations(&self) -> Vec<String> {
        ["where", "echo", "first", "spin", "result"]
            .map(String::from)
            .to_vec()
    }

    fn invoke(
        &self,
        mut context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        Box::pin(async move {
            let mut exit_code = 0;
            match context.request.operation.as_str() {
                "where" => {
                    let value = serde_json::json!({
                        "label": context.request.args.get("label"),
                        "cwd": context.request.cwd,
                        "value": context.request.env.get("PROBE"),
                    });
                    context
                        .output
                        .stdout(Bytes::from(value.to_string()))
                        .await?;
                }
                "echo" | "first" => {
                    while let Some(bytes) = context.input.next().await? {
                        context.output.stdout(bytes).await?;
                        if context.request.operation == "first" {
                            break;
                        }
                    }
                }
                "spin" => {
                    context
                        .output
                        .stdout(Bytes::from_static(b"started"))
                        .await?;
                    tokio::task::spawn_blocking(|| {
                        loop {
                            std::hint::spin_loop();
                        }
                    })
                    .await
                    .map_err(|error| ServiceError::Handler(error.to_string()))?;
                }
                "result" => {
                    context
                        .output
                        .stdout(Bytes::from_static(b"command output"))
                        .await?;
                    context
                        .output
                        .stderr(Bytes::from_static(b"command diagnostic"))
                        .await?;
                    match context.request.env.get("RESULT").map(String::as_str) {
                        Some("error") => {
                            return Err(ServiceError::Handler("command failed".into()));
                        }
                        _ => exit_code = 17,
                    }
                }
                _ => return Err(ServiceError::Handler("unknown fixture operation".into())),
            }
            Ok(Completion {
                exit_code,
                error: None,
            })
        })
    }
}

#[tokio::main]
async fn main() {
    let result = demi_command_service::serve_stdio(Arc::new(Fixture)).await;
    if let Err(error) = result {
        eprintln!("fixture: {error}");
        std::process::exit(1);
    }
    std::process::exit(0);
}
