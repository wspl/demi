//! Local Host command fixtures exercise the production resource dispatch.

use futures_util::FutureExt;
use std::{collections::BTreeMap, future::Future, sync::Arc};

use demi_command_service::{
    Handler, Input, InvocationContext, Output, ServiceError,
    protocol::{Invocation, NativeResourceScope, Record},
};
use demi_commands::DemiCommands;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
pub struct BrowserFixture {
    service: Arc<DemiCommands>,
    pub root: Arc<tempfile::TempDir>,
    scope: NativeResourceScope,
    pub caller: String,
}

impl BrowserFixture {
    pub async fn call(&self, operation: &str, args: Value) -> Value {
        let (code, result) = self.result(operation, args, CancellationToken::new()).await;
        assert_eq!(code, 0, "{operation}: {result}");
        result
    }

    pub async fn result(
        &self,
        operation: &str,
        args: Value,
        cancel: CancellationToken,
    ) -> (u8, Value) {
        self.result_with_input(operation, args, cancel, Vec::new())
            .await
    }

    pub async fn result_with_input(
        &self,
        operation: &str,
        args: Value,
        cancel: CancellationToken,
        bytes: Vec<u8>,
    ) -> (u8, Value) {
        let (output, mut records) = Output::channel(CancellationToken::new());
        let context = InvocationContext {
            request: Invocation {
                operation: operation.into(),
                invocation_id: uuid::Uuid::new_v4().to_string(),
                caller: Some(self.caller.clone()),
                resource: Some(self.scope.clone()),
                json: Some(true),
                edits: None,
                args,
                cwd: self.root.path().to_str().unwrap().into(),
                env: BTreeMap::new(),
            },
            input: Input::from_stream(futures_util::stream::iter([Ok(bytes::Bytes::from(bytes))])),
            output,
            cancellation: cancel,
        };
        let invoke = async {
            if operation.starts_with("browser.") {
                self.service.invoke(context).await
            } else {
                self.service.resource(context).await
            }
        };
        let collect = async {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            while let Some(record) = records.recv().await {
                match record {
                    Record::Stdout(bytes) => stdout.extend_from_slice(&bytes),
                    Record::Stderr(bytes) => stderr.extend_from_slice(&bytes),
                    _ => panic!("unexpected fixture output record"),
                }
            }
            (stdout, stderr)
        };
        let (completion, (stdout, stderr)) = tokio::join!(invoke, collect);
        let completion = match completion {
            Ok(completion) => completion,
            Err(ServiceError::Cancelled) => {
                assert!(stdout.is_empty() && stderr.is_empty());
                // The command service reports bare cancellation as a typed
                // transport error; represent that result for fixture assertions.
                return (130, json!({"error":{"code":"cancelled"}}));
            }
            Err(error) => panic!("{operation}: {error}"),
        };
        let bytes = if completion.exit_code == 0 {
            stdout
        } else {
            stderr
        };
        let value = serde_json::from_slice(&bytes)
            .unwrap_or_else(|_| json!({"diagnostic": String::from_utf8_lossy(&bytes)}));
        (completion.exit_code, value)
    }

    pub async fn open(&self, name: &str) -> String {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/browser")
            .join(name);
        let url = url::Url::from_file_path(path).unwrap();
        self.call(
            "browser.open",
            json!({"url": url.as_str(), "timeout": 120000}),
        )
        .await["tab"]
            .as_str()
            .unwrap()
            .into()
    }
}

pub async fn with_browser_fixture<F, W>(exercise: F)
where
    F: FnOnce(BrowserFixture) -> W,
    W: Future<Output = BrowserFixture>,
{
    let fixture = BrowserFixture {
        service: Arc::new(DemiCommands::default()),
        root: Arc::new(tempfile::tempdir().unwrap()),
        caller: "browser-family-test".into(),
        scope: NativeResourceScope {
            id: uuid::Uuid::new_v4().to_string(),
            kind: "browser".into(),
        },
    };
    fixture.call("acquire", json!({})).await;
    let result = std::panic::AssertUnwindSafe(exercise(fixture.clone()))
        .catch_unwind()
        .await;
    fixture
        .service
        .close()
        .await
        .expect("release fixture browser even after an assertion failure");
    if let Err(panic) = result {
        std::panic::resume_unwind(panic);
    }
}
