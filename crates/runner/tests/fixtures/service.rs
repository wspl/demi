//! Deliberately faulty operations available only in native integration tests.
use bytes::Bytes;
use demi_command_service::protocol::{Completion, ConversationRequest, Invocation};
use demi_command_service::{ConversationContext, Handler, InvocationContext, ServiceError};
use std::{
    collections::BTreeSet,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Fixture {
    conversations: Arc<Mutex<BTreeSet<String>>>,
}

impl Handler for Fixture {
    type Metadata = Invocation;

    fn operations(&self) -> Vec<String> {
        ["where", "echo", "first", "spin", "result", "retain", "crash"]
            .map(String::from)
            .to_vec()
    }

    fn invoke(
        &self,
        mut context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        let conversations = self.conversations.clone();
        Box::pin(async move {
            let mut exit_code = 0;
            match context.request.operation.as_str() {
                "retain" => {
                    conversations
                        .lock()
                        .unwrap()
                        .insert(context.request.context.conversation);
                }
                // The whole service fails, leaving its last words on
                // standard error.
                "crash" => {
                    eprintln!("fixture crashing on purpose");
                    std::process::exit(3);
                }
                "where" => {
                    let value = serde_json::json!({
                        "label": context.request.args.get("label"),
                        "context": context.request.context,
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
                    .await?;
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
                            return Err(ServiceError::failed("command failed"));
                        }
                        _ => exit_code = 17,
                    }
                }
                operation => return Err(ServiceError::UnknownOperation(operation.into())),
            }
            Ok(Completion {
                exit_code,
                error: None,
            })
        })
    }
    fn conversation(
        &self,
        context: ConversationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        let conversations = self.conversations.clone();
        Box::pin(async move {
            let value = {
                let mut held = conversations.lock().unwrap();
                match &context.request {
                    // A service that cannot say what it holds.
                    ConversationRequest::Status {} if held.contains("unanswerable") => {
                        return Err(ServiceError::failed("fixture status unavailable"));
                    }
                    ConversationRequest::Status {} => {
                        serde_json::json!({ "conversations": *held })
                    }
                    ConversationRequest::Release { conversation } if conversation == "fail" => {
                        return Err(ServiceError::failed("fixture cleanup failed"));
                    }
                    ConversationRequest::Release { conversation } => {
                        held.remove(conversation);
                        serde_json::json!({})
                    }
                }
            };
            context.output.stdout(value.to_string().into()).await?;
            Ok(Completion {
                exit_code: 0,
                error: None,
            })
        })
    }
}

#[tokio::main]
async fn main() {
    let result = demi_command_service::serve_stdio(Arc::new(Fixture::default())).await;
    if let Err(error) = result {
        eprintln!("fixture: {error}");
        std::process::exit(1);
    }
    std::process::exit(0);
}
