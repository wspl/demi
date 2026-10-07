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

/// The exit code and the JSON that `operation` with `args` prints, on
/// standard output or on standard error, for `caller` while no browser runs.
async fn invoked_for(
    caller: CommandCaller,
    operation: &str,
    args: serde_json::Value,
) -> (u8, serde_json::Value) {
    let service = DemiBrowser::new();
    let cancel = CancellationToken::new();
    let (output, mut records) = Output::channel(cancel.clone());
    let completion = service
        .invoke(InvocationContext {
            request: Invocation {
                operation: operation.into(),
                invocation_id: "listed".into(),
                context: CommandContext {
                    conversation: "conversation".into(),
                    caller,
                    locale: CommandLocale {
                        time_zone: "UTC".into(),
                        languages: vec!["en-US".into()],
                    },
                },
                args,
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
    // The output closes with the invocation.
    let mut printed = Vec::new();
    while let Some(record) = records.recv().await {
        if let Record::Stdout(bytes) | Record::Stderr(bytes) = record {
            printed.extend_from_slice(&bytes);
        }
    }
    service.close().await.unwrap();
    (completion.exit_code, serde_json::from_slice(&printed).unwrap())
}

/// The JSON `browser.tabs` prints for `caller` while no browser runs.
async fn listed_for(caller: CommandCaller) -> serde_json::Value {
    let (code, listed) = invoked_for(caller, "browser.tabs", json!({})).await;
    assert_eq!(code, 0, "{listed}");
    listed
}

/// The tab list's number and its tabs closed on purpose are the page's: the
/// agent's `demi browser tabs --json` never carries them (`runtime.md`,
/// "Only what the model uses").
#[tokio::test]
async fn only_the_users_tab_list_names_its_number() {
    assert_eq!(
        listed_for(CommandCaller::agent(1)).await,
        json!({"tabs": [], "truncated": false})
    );
    assert_eq!(
        listed_for(CommandCaller::User {}).await,
        json!({"list": 0, "tabs": [], "truncated": false, "closed": []})
    );
}

/// Stop is the user's alone (`live-view.md` § The tab methods): an agent's
/// call is refused before it reaches any browser, while the user's reaches
/// the browser, which has no such tab here.
#[tokio::test]
async fn only_the_user_stops_a_load() {
    let tab = json!({"tab": "t1"});
    let (code, refused) = invoked_for(CommandCaller::agent(1), "browser.stop", tab.clone()).await;
    assert_eq!((code, &refused["error"]["code"]), (2, &json!("invalid_input")));
    let (code, missing) = invoked_for(CommandCaller::User {}, "browser.stop", tab).await;
    assert_eq!((code, &missing["error"]["code"]), (1, &json!("tab_not_found")));
}
