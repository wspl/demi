//! Deliberately faulty operations available only in native integration tests.
use bytes::Bytes;
use demi_command_protocol::{Completion, ConversationRequest, Invocation, ServiceSequence};
use demi_command_service::{
    ConversationContext, Handler, InvocationContext, Numbers, ServiceError,
};
use std::{
    collections::BTreeSet,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};
use tokio::sync::Notify;

#[derive(Default)]
struct Fixture {
    conversations: Arc<Mutex<BTreeSet<String>>>,
    /// The conversations whose release never ends by itself.
    stalling: Arc<Mutex<BTreeSet<String>>>,
    /// Told when a status taken while `stall` is held, or a release of a
    /// stalling conversation, waits to answer.
    stalled: Arc<Notify>,
    /// Lets that status answer, with what it held when it was asked.
    proceed: Arc<Notify>,
    /// The connection's numbers source.
    numbers: Arc<Mutex<Option<Numbers>>>,
}

impl Handler for Fixture {
    type Metadata = Invocation;

    fn operations(&self) -> Vec<String> {
        demi_command_protocol::testing::FIXTURE_OPERATIONS
            .map(String::from)
            .to_vec()
    }

    fn numbers(&self, numbers: Numbers) {
        *self.numbers.lock().unwrap() = Some(numbers);
    }

    fn invoke(
        &self,
        mut context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        let (conversations, stalling) = (self.conversations.clone(), self.stalling.clone());
        let (stalled, proceed) = (self.stalled.clone(), self.proceed.clone());
        let numbers = self.numbers.lock().unwrap().clone();
        Box::pin(async move {
            let mut exit_code = 0;
            match context.request.operation.as_str() {
                // Draws `count` tab numbers of the invoking conversation, one
                // by default, and prints the first.
                "number" => {
                    let numbers = numbers.ok_or_else(|| ServiceError::failed("no numbers source"))?;
                    let count = context.request.args.get("count").and_then(|count| count.as_u64()).unwrap_or(1);
                    let first = numbers
                        .draw(
                            &context.request.context.conversation,
                            ServiceSequence::Tab,
                            u32::try_from(count).map_err(ServiceError::failed)?,
                        )
                        .await?;
                    let value = serde_json::json!({ "first": first });
                    context
                        .output
                        .stdout(Bytes::from(value.to_string()))
                        .await?;
                }
                // Ends once a status or a release waits to answer.
                "stalled" => stalled.notified().await,
                "proceed" => proceed.notify_one(),
                "retain" => {
                    conversations
                        .lock()
                        .unwrap()
                        .insert(context.request.context.conversation);
                }
                // Holds the conversation as `retain` does, and its release
                // then never ends by itself, as one that a lost connection
                // cuts off: it ends with the runner's connection to the
                // service.
                "stall_release" => {
                    let conversation = context.request.context.conversation;
                    conversations.lock().unwrap().insert(conversation.clone());
                    stalling.lock().unwrap().insert(conversation);
                }
                // What the service holds, as its status answers it, for a
                // test that watches a release end a conversation's state.
                "held" => {
                    let held = conversations.lock().unwrap().clone();
                    let value = serde_json::json!({ "conversations": held });
                    context
                        .output
                        .stdout(Bytes::from(value.to_string()))
                        .await?;
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
        let (conversations, stalling) = (self.conversations.clone(), self.stalling.clone());
        let (stalled, proceed) = (self.stalled.clone(), self.proceed.clone());
        Box::pin(async move {
            if let ConversationRequest::Release { conversation } = &context.request
                && stalling.lock().unwrap().contains(conversation)
            {
                stalled.notify_one();
                context.cancellation.cancelled().await;
                return Err(ServiceError::Cancelled);
            }
            let (value, stall) = {
                let mut held = conversations.lock().unwrap();
                match &context.request {
                    // A service that cannot say what it holds.
                    ConversationRequest::Status {} if held.contains("unanswerable") => {
                        return Err(ServiceError::failed("fixture status unavailable"));
                    }
                    // What it holds is read at once; while it holds `stall`,
                    // the answer arrives only after `proceed`, as a late one
                    // does.
                    ConversationRequest::Status {} => (
                        serde_json::json!({ "conversations": *held }),
                        held.contains("stall"),
                    ),
                    ConversationRequest::Release { conversation } if conversation == "fail" => {
                        return Err(ServiceError::failed("fixture cleanup failed"));
                    }
                    ConversationRequest::Release { conversation } => {
                        held.remove(conversation);
                        (serde_json::json!({}), false)
                    }
                }
            };
            if stall {
                stalled.notify_one();
                proceed.notified().await;
            }
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
