//! The messages stream (`crates/command-protocol/src/messages.rs`)
//! between a service's handler and the end that takes its messages.

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use demi_command_protocol::{CommandCaller, CommandContext, CommandLocale, Completion, Invocation, Record};
use demi_command_sdk::{Client, Handler, InvocationContext, Messages, MessagesAsk, ServiceError, serve};
use serde_json::{Value, json};
use tokio::sync::mpsc;

/// Sends the messages its call names, one after another, and prints `sent`
/// once every one was taken, or the error of the first that was not.
#[derive(Default)]
struct Telling {
    messages: Mutex<Option<Messages>>,
}

impl Handler for Telling {
    type Metadata = Invocation;

    fn operations(&self) -> Vec<String> {
        vec!["tell".into()]
    }

    fn messages(&self, messages: Messages) {
        *self.messages.lock().unwrap() = Some(messages);
    }

    fn invoke(
        &self,
        context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        let messages = self.messages.lock().unwrap().clone();
        Box::pin(async move {
            let messages = messages.expect("the service takes its sender before any call");
            let conversation = context.request.context.conversation.clone();
            let mut printed = "sent".to_owned();
            for message in context.request.args["messages"].as_array().unwrap() {
                if let Err(error) = messages.send(&conversation, message.clone()).await {
                    printed = error.to_string();
                    break;
                }
            }
            context.output.stdout(printed.into()).await?;
            Ok(Completion {
                exit_code: 0,
                error: None,
            })
        })
    }
}

/// What a `tell` call of `messages` for conversation `c1` printed.
async fn tell(client: &Client, messages: Vec<Value>) -> String {
    let (mut input, mut output) = client
        .invoke(&Invocation {
            operation: "tell".into(),
            invocation_id: "tell".into(),
            command: "demi test".into(),
            context: CommandContext {
                color_scheme: demi_command_protocol::ColorScheme::Light,
                conversation: "c1".into(),
                caller: CommandCaller::agent(0),
                locale: CommandLocale {
                    time_zone: "UTC".into(),
                    languages: vec!["en-US".into()],
                },
            },
            args: json!({ "messages": messages }),
            cwd: "/".into(),
            env: Default::default(),
            edits: None,
            json: None,
            live_input: None,
            viewable: None,
        })
        .await
        .unwrap();
    input.end().unwrap();
    let mut printed = Vec::new();
    while let Some(record) = output.next().await.unwrap() {
        match record {
            Record::Stdout(bytes) => printed.extend_from_slice(&bytes),
            Record::Completion(completion) => assert_eq!(completion.exit_code, 0, "{completion:?}"),
            other => panic!("unexpected record {other:?}"),
        }
    }
    String::from_utf8(printed).unwrap()
}

#[tokio::test]
async fn messages_reach_the_taking_end_in_order_and_one_beyond_a_record_is_refused_where_it_is_sent() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let handler = Arc::new(Telling::default());
        let (io, service) = tokio::io::duplex(256 * 1024);
        let server = tokio::spawn(serve(service, handler));
        let (client, connection) = Client::connect(io).await.unwrap();
        let driver = tokio::spawn(connection);
        let stream = client.messages().await.unwrap();
        let (taken, mut took) = mpsc::unbounded_channel();
        let taking = tokio::spawn(async move {
            stream
                .answer::<MessagesAsk, _>(|message, _reporter| {
                    let _kept = taken.send((message.conversation, message.message));
                    std::future::ready(Ok(()))
                })
                .await
        });
        let sent = vec![json!({ "tabs": "changed" }), json!(2), json!("three")];
        assert_eq!(tell(&client, sent.clone()).await, "sent");
        for message in sent {
            assert_eq!(took.recv().await.unwrap(), ("c1".to_owned(), message));
        }
        // A message that cannot fit one record never reaches the stream.
        let large = json!("x".repeat(70 * 1024));
        assert!(tell(&client, vec![large]).await.contains("exceeds limit"));
        client.shutdown().await.unwrap();
        taking.await.unwrap().unwrap();
        assert!(took.try_recv().is_err());
        server.await.unwrap().unwrap();
        drop(client);
        driver.await.unwrap().unwrap();
    })
    .await
    .unwrap();
}
