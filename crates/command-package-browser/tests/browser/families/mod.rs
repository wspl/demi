//! Local Host command fixtures exercise the production conversation dispatch.

use futures_util::FutureExt;
use std::{collections::BTreeMap, future::Future, sync::Arc};

use demi_browser::DemiBrowser;
use demi_command_protocol::{
    CommandCaller, CommandContext, CommandLocale, Completion, Invocation, Record, StdoutTarget,
};
use demi_command_sdk::{Handler, Input, InvocationContext, Output, ServiceError};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

/// An invocation's output records and the future that runs it.
pub type Invoked = (
    tokio::sync::mpsc::Receiver<Record>,
    std::pin::Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>>,
);

#[derive(Clone)]
pub struct BrowserFixture {
    service: Arc<DemiBrowser>,
    /// The browser directory the service finds Chrome in, until it ends.
    pub root: Arc<tempfile::TempDir>,
    pub conversation: String,
    pub env: BTreeMap<String, String>,
    /// The number of the agent the fixture's commands run for.
    pub caller: u64,
    /// The locale the invocations carry; the browser starts in the first one's.
    pub locale: CommandLocale,
    /// Whether invocations ask for JSON, as `--json` does; text arrives as
    /// the answer's `diagnostic`.
    pub json: bool,
    /// Where the invocations' stdout goes, for invocations a job's command
    /// makes, which may return media; none for any other.
    pub stdout: Option<StdoutTarget>,
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

    /// Starts an invocation for `caller` reading `input`; its output records
    /// arrive on the returned queue.
    pub fn start(
        &self,
        operation: &str,
        args: Value,
        caller: CommandCaller,
        input: Input,
        cancel: CancellationToken,
    ) -> Invoked {
        let (mut output, records) = Output::channel(CancellationToken::new());
        if self.stdout.is_some() {
            output = output.returning_media();
        }
        let context = InvocationContext {
            request: Invocation {
                operation: operation.into(),
                invocation_id: uuid::Uuid::new_v4().to_string(),
                context: CommandContext {
                    conversation: self.conversation.clone(),
                    caller,
                    locale: self.locale.clone(),
                },
                json: Some(self.json),
                edits: None,
                args,
                cwd: self.root.path().to_str().unwrap().into(),
                env: self.env.clone(),
                stdout: self.stdout,
            },
            input,
            output,
            cancellation: cancel,
        };
        (records, self.service.invoke(context))
    }

    pub async fn result_with_input(
        &self,
        operation: &str,
        args: Value,
        cancel: CancellationToken,
        bytes: Vec<u8>,
    ) -> (u8, Value) {
        let caller = CommandCaller::agent(self.caller);
        self.result_for(caller, operation, args, cancel, bytes)
            .await
    }

    /// The exit code and JSON answer of `operation` run for `caller`.
    pub async fn result_for(
        &self,
        caller: CommandCaller,
        operation: &str,
        args: Value,
        cancel: CancellationToken,
        bytes: Vec<u8>,
    ) -> (u8, Value) {
        let (mut records, invoke) = self.start(
            operation,
            args,
            caller,
            Input::from_stream(futures_util::stream::iter([Ok(bytes::Bytes::from(bytes))])),
            cancel,
        );
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

    pub async fn lifecycle(&self, operation: &str) -> Value {
        use demi_command_protocol::ConversationRequest;
        let request = match operation {
            "status" => ConversationRequest::Status {},
            "release" => ConversationRequest::Release {
                conversation: self.conversation.clone(),
            },
            other => panic!("unknown lifecycle operation {other}"),
        };
        let (output, mut records) = Output::channel(CancellationToken::new());
        let invoke = self
            .service
            .conversation(demi_command_sdk::ConversationContext {
                request,
                output,
                cancellation: CancellationToken::new(),
            });
        let collect = async {
            let mut stdout = Vec::new();
            while let Some(record) = records.recv().await {
                match record {
                    Record::Stdout(bytes) => stdout.extend_from_slice(&bytes),
                    _ => panic!("unexpected lifecycle output"),
                }
            }
            serde_json::from_slice(&stdout).unwrap()
        };
        let (completion, value) = tokio::join!(invoke, collect);
        assert_eq!(completion.unwrap().exit_code, 0);
        value
    }

    /// Waits until a concurrent command holds `tab`'s operation lock.
    pub async fn wait_until_busy(&self, tab: &str) {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                let (_, result) = self
                    .result(
                        "browser.info",
                        json!({"tab": tab}),
                        CancellationToken::new(),
                    )
                    .await;
                if result["error"]["code"] == "tab_busy" {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the tab becomes busy");
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

/// The answer a runner that has installed the pinned Chrome for Testing
/// gives the browser's requests: the copy `DEMI_TEST_CHROME` names. No test
/// downloads Chrome.
pub fn answer_chrome(
    ask: demi_command_protocol::ArtifactAsk,
) -> Result<demi_command_protocol::ArtifactReply, String> {
    let chrome = std::env::var("DEMI_TEST_CHROME").expect("DEMI_TEST_CHROME");
    Ok(match ask {
        demi_command_protocol::ArtifactAsk::Install(_) => {
            demi_command_protocol::ArtifactReply::Path(chrome)
        }
        demi_command_protocol::ArtifactAsk::Installed(_) => {
            let pinned = demi_command_package_browser_protocol::release::BrowserRelease::pinned()
                .expect("the pinned release");
            let platform = pinned
                .platform(demi_command_protocol::host_target())
                .expect("the pinned release has this machine's target");
            demi_command_protocol::ArtifactReply::Installed(vec![
                demi_command_protocol::InstalledArtifact {
                    version: pinned.version.clone(),
                    sha256: platform.sha256.clone(),
                    path: chrome,
                },
            ])
        }
    })
}

/// The service, whose requests for Chrome [`answer_chrome`] answers.
pub fn test_browser() -> DemiBrowser {
    let service = DemiBrowser::new();
    service.artifacts(demi_command_sdk::testing::artifacts_from(answer_chrome));
    service
}

pub async fn with_browser_fixture<F, W>(exercise: F)
where
    F: FnOnce(BrowserFixture) -> W,
    W: Future<Output = BrowserFixture>,
{
    with_numbered_browser_fixture(demi_command_sdk::testing::counting_numbers(), exercise).await;
}

/// [`with_browser_fixture`] whose service draws its tab numbers from
/// `numbers`, as from the runner's numbers stream.
pub async fn with_numbered_browser_fixture<F, W>(numbers: demi_command_sdk::Numbers, exercise: F)
where
    F: FnOnce(BrowserFixture) -> W,
    W: Future<Output = BrowserFixture>,
{
    let service = test_browser();
    service.numbers(numbers);
    let fixture = BrowserFixture {
        service: Arc::new(service),
        root: Arc::new(tempfile::tempdir().unwrap()),
        caller: 1,
        conversation: uuid::Uuid::new_v4().to_string(),
        env: BTreeMap::new(),
        locale: CommandLocale {
            time_zone: "UTC".into(),
            languages: vec!["en-US".into()],
        },
        json: true,
        stdout: None,
    };
    // Catch both construction and polling of the exercise, including the initial
    // service assertions, before joining retirement and resuming the same panic.
    let result = std::panic::AssertUnwindSafe(async {
        assert_eq!(
            fixture.lifecycle("status").await,
            json!({"conversations": []})
        );
        exercise(fixture.clone()).await
    })
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
