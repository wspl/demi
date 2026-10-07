use bytes::Bytes;
use demi_browser::DemiBrowser;
use demi_command_protocol::{
    CommandCaller, CommandContext, CommandLocale, ConversationRequest, Invocation, Record,
};
use demi_command_sdk::{
    ConversationContext, Handler, Input, InvocationContext, Output, ServiceError,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn release_cancels_a_browser_command_blocked_on_output() {
    let service = DemiBrowser::new();
    let cancel = CancellationToken::new();
    let (output, mut records) = Output::channel(cancel.clone());
    for _ in 0..4 {
        output
            .stdout(Bytes::from_static(b"occupied"))
            .await
            .unwrap();
    }
    let command = service.invoke(InvocationContext {
        request: Invocation {
            operation: "browser.tabs".into(),
            invocation_id: "blocked-output".into(),
            context: CommandContext {
                conversation: "conversation".into(),
                caller: CommandCaller::agent(1),
                locale: CommandLocale {
                    time_zone: "UTC".into(),
                    languages: vec!["en-US".into()],
                },
            },
            args: json!({}),
            cwd: "/".into(),
            env: Default::default(),
            edits: None,
            json: Some(true),
            stdout: None,
        },
        input: Input::from_stream(futures_util::stream::empty()),
        output,
        cancellation: cancel,
    });
    tokio::pin!(command);
    // An empty tabs invocation reaches the full output queue without launching Chrome.
    assert!(futures_util::poll!(command.as_mut()).is_pending());
    let (output, mut release_records) = Output::channel(CancellationToken::new());
    let release = service.conversation(ConversationContext {
        request: ConversationRequest::Release {
            conversation: "conversation".into(),
        },
        output,
        cancellation: CancellationToken::new(),
    });
    let (command, release) = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        tokio::join!(command, release)
    })
    .await
    .unwrap();
    assert!(matches!(command, Err(ServiceError::Cancelled)));
    assert_eq!(release.unwrap().exit_code, 0);
    assert!(matches!(release_records.recv().await, Some(Record::Stdout(bytes)) if bytes == "{}"));
    for _ in 0..4 {
        assert!(matches!(records.recv().await, Some(Record::Stdout(bytes)) if bytes == "occupied"));
    }
    assert!(records.recv().await.is_none());
    service.close().await.unwrap();
}

/// The JSON `browser.tabs` prints for `caller` while no browser runs.
async fn listed_for(caller: CommandCaller) -> serde_json::Value {
    let service = DemiBrowser::new();
    let cancel = CancellationToken::new();
    let (output, mut records) = Output::channel(cancel.clone());
    let completion = service
        .invoke(InvocationContext {
            request: Invocation {
                operation: "browser.tabs".into(),
                invocation_id: "listed".into(),
                context: CommandContext {
                    conversation: "conversation".into(),
                    caller,
                    locale: CommandLocale {
                        time_zone: "UTC".into(),
                        languages: vec!["en-US".into()],
                    },
                },
                args: json!({}),
                cwd: "/".into(),
                env: Default::default(),
                edits: None,
                json: Some(true),
                stdout: None,
            },
            input: Input::from_stream(futures_util::stream::empty()),
            output,
            cancellation: cancel,
        })
        .await
        .unwrap();
    assert_eq!(completion.exit_code, 0);
    // The output closes with the invocation.
    let mut stdout = Vec::new();
    while let Some(record) = records.recv().await {
        if let Record::Stdout(bytes) = record {
            stdout.extend_from_slice(&bytes);
        }
    }
    service.close().await.unwrap();
    serde_json::from_slice(&stdout).unwrap()
}

/// The tab list's number is the page's: the agent's `demi browser tabs --json`
/// never carries it (`runtime.md`, "Only what the model uses").
#[tokio::test]
async fn only_the_users_tab_list_names_its_number() {
    assert_eq!(
        listed_for(CommandCaller::agent(1)).await,
        json!({"tabs": [], "truncated": false})
    );
    assert_eq!(
        listed_for(CommandCaller::User {}).await,
        json!({"list": 0, "tabs": [], "truncated": false})
    );
}
