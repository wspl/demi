use bytes::Bytes;
use demi_command_service::{
    ConversationContext, Handler, Input, InvocationContext, Output, ServiceError,
    protocol::{ConversationRequest, Invocation, Record},
};
use demi_commands::DemiCommands;
use serde_json::json;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn release_cancels_a_browser_command_blocked_on_output() {
    let service = DemiCommands::default();
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
            conversation: "conversation".into(),
            caller: "caller".into(),
            args: json!({}),
            cwd: "/".into(),
            env: Default::default(),
            edits: None,
            json: Some(true),
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
        request: ConversationRequest {
            operation: "release".into(),
            conversation: Some("conversation".into()),
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
