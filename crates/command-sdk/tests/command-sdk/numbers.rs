//! The numbers stream (`native-runtime.md` § Conversation numbers) between a
//! service's handler and the end that answers it.

use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use bytes::Bytes;
use demi_command_protocol::{
    CommandCaller, CommandContext, CommandLocale, Completion, Invocation, Record, ServiceSequence,
};
use demi_command_sdk::{
    Client, Handler, InvocationContext, Numbers, ServiceError, serve, testing::answer_numbers,
};
use tokio::sync::Notify;

/// Draws `count` tab numbers of the invoking conversation and prints the
/// first; tells `drawing` as it asks.
#[derive(Default)]
struct Drawing {
    numbers: Mutex<Option<Numbers>>,
    drawing: Arc<Notify>,
}

impl Handler for Drawing {
    type Metadata = Invocation;

    fn operations(&self) -> Vec<String> {
        vec!["draw".into()]
    }

    fn numbers(&self, numbers: Numbers) {
        *self.numbers.lock().unwrap() = Some(numbers);
    }

    fn invoke(
        &self,
        context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        let numbers = self.numbers.lock().unwrap().clone();
        let drawing = self.drawing.clone();
        Box::pin(async move {
            let numbers = numbers.expect("the service takes its numbers before any call");
            let count = context.request.args["count"].as_u64().unwrap() as u32;
            let conversation = &context.request.context.conversation;
            let draw = numbers.draw(conversation, ServiceSequence::Tab, count);
            drawing.notify_one();
            let first = draw.await?;
            context
                .output
                .stdout(Bytes::from(first.to_string()))
                .await?;
            Ok(Completion {
                exit_code: 0,
                error: None,
            })
        })
    }
}

/// The first of `count` numbers a `draw` call printed for `conversation`.
async fn draw(client: &Client, conversation: &str, count: u32) -> String {
    let (mut input, mut output) = client
        .invoke(&Invocation {
            operation: "draw".into(),
            invocation_id: "draw".into(),
            context: CommandContext {
                color_scheme: demi_command_protocol::ColorScheme::Light,
                conversation: conversation.into(),
                caller: CommandCaller::agent(0),
                locale: CommandLocale {
                    time_zone: "UTC".into(),
                    languages: vec!["en-US".into()],
                },
            },
            args: serde_json::json!({ "count": count }),
            cwd: "/".into(),
            env: Default::default(),
            edits: None,
            json: None,
            stdout: None,
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
async fn draws_wait_for_the_numbers_stream_go_on_per_conversation_and_shutdown_ends_it() {
    tokio::time::timeout(Duration::from_secs(10), async {
        let handler = Arc::new(Drawing::default());
        let (io, service) = tokio::io::duplex(64 * 1024);
        let server = tokio::spawn(serve(service, handler.clone()));
        let (client, connection) = Client::connect(io).await.unwrap();
        let driver = tokio::spawn(connection);
        // A draw made before the stream opens waits for it.
        let drawing = handler.drawing.notified();
        let early = tokio::spawn({
            let client = client.clone();
            async move { draw(&client, "one", 4).await }
        });
        drawing.await;
        let answering = answer_numbers(&client).await.unwrap();
        assert_eq!(early.await.unwrap(), "1");
        // Each conversation's numbers go on from its last draw.
        assert_eq!(draw(&client, "one", 1).await, "5");
        assert_eq!(draw(&client, "two", 1).await, "1");
        // The stream opens once.
        assert!(matches!(
            client.numbers().await,
            Err(ServiceError::Rejected(409))
        ));
        // Shutdown completes the stream, so the connection drains and the
        // service ends.
        client.shutdown().await.unwrap();
        answering.await.unwrap().unwrap();
        server.await.unwrap().unwrap();
        drop(client);
        driver.await.unwrap().unwrap();
    })
    .await
    .unwrap();
}
