//! The real Claude Code CLI through the backend
//! (`crates/provider-claude-code/src/lib.rs`).
//!
//! The Claude Code machine installs Demi's copy of the CLI from a local
//! distribution and verifies it, and a conversation infers through it as in
//! the product.
//!
//! Scenarios:
//!
//! - The CLI declares Demi's SDK MCP server and offers the model its tools,
//!   streams reasoning and text, runs a batch of tool calls through Demi and
//!   sends back their results, reports usage, stops in the middle of a stream,
//!   and reports a vendor error as the request's failure.
//! - It prints back each user message it takes, before the `result` that
//!   answers it, and answers a command it does not offer itself, never
//!   printing it back.
//! - Each new process, for another model and effort, after another provider's
//!   turn or after a stop, resumes the session the blocks hold: its first
//!   request begins with the last request of the process before it, with no
//!   account of the CLI's machine.
//! - It reads the account's sign-in, which an earlier release's record handed
//!   to the account's directory, and runs in a directory of its own under it
//!   that goes with it.
//!
//! Every assertion is on what the product observes: the transcript, the
//! frames, the usage ledger and what the vendor received. No scenario has the
//! vendor refuse the sign-in: the CLI would then renew it at Anthropic's own
//! token endpoint.
//!
//! # Running the suite
//!
//! The suite runs only when `DEMI_TEST_CLAUDE_CODE` names the CLI's
//! executable, as an ordinary user, since the CLI refuses the provider's
//! permission mode as root, with `SSL_CERT_FILE` naming
//! `claude_code/distribution-ca.pem`.
//!
//! ```sh
//! DEMI_TEST_CLAUDE_CODE=<claude> \
//! SSL_CERT_FILE=$PWD/crates/backend/tests/backend/claude_code/distribution-ca.pem \
//!   cargo test --workspace --features demi-runner/test-fixtures --test backend \
//!   -- --ignored claude_code
//! ```
//!
//! # Isolation from the vendor
//!
//! Nothing reaches the vendor's service:
//!
//! - The CLI's inference goes to the scripted vendor on 127.0.0.1, which the
//!   Cloud's runner names in `ANTHROPIC_BASE_URL`.
//! - The runner's environment is the suite's own, so no proxy of the
//!   machine's reaches the CLI.
//! - The CLI's non-essential traffic, telemetry and error reporting are off.
//! - The account's tokens are made up. The account is stored as a sign-in
//!   would store it, since the CLI's own sign-in reaches only the vendor.
//!
//! # The distribution's certificate
//!
//! The distribution serves HTTPS, since `demi.claude-code` downloads nothing
//! else, with the certificate in `claude_code/`. The backend in this process
//! and the Cloud's `demi.claude-code` trust its CA through `SSL_CERT_FILE`,
//! which the machine's roots of the one selection's reqwest read; a selection
//! without them fails the install with an unknown issuer. The CA's key was
//! discarded, and its certificate and the distribution's expire on 2126-09-03.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use bytes::Bytes;
use demi_conversation_socket_protocol::ServerFrame;
use demi_provider_common::testing::{MockResponse, MockVendor, RecordedRequest};
use demi_shared_types::{Block, TokenUsage, ToolCallStatus, ToolResultContentBlock};
use demi_web_api_protocol::providers::{CliInstall, NewestVersion};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::claude::{PackageService, claude_models, cli, settled};
use crate::conversations::{
    Socket, anthropic, choose, create, events, kinds, last_text, message, message_start, send,
    text_block, thinking_block, tool_use_block, transcript, usage,
};
use crate::support::{Harness, Session, TestBackend};

/// The account's access token: made up, since no request reaches the
/// vendor.
const TOKEN: &str = "sk-ant-oat01-demi-suite-made-up-token";
const CONVERSATION: &str = "4d2e3f5a-9b4c-4d2f-8e3a-6b2d3f4a5c01";
/// The conversation's model, and the one it changes to.
const MODEL: &str = "claude-opus-4-8";
const OTHER_MODEL: &str = "claude-sonnet-4-6";
/// Where the distribution keeps its releases.
const RELEASES: &str = "/claude-code-releases";
/// The platforms of the vendor's releases. The distribution offers the
/// supplied executable under each, since the machine that runs the suite
/// takes the one it is.
const PLATFORMS: [&str; 8] = [
    "darwin-arm64",
    "darwin-x64",
    "linux-arm64",
    "linux-arm64-musl",
    "linux-x64",
    "linux-x64-musl",
    "win32-arm64",
    "win32-x64",
];
/// The suite's CA, which `SSL_CERT_FILE` names, and the distribution's
/// certificate for 127.0.0.1 with its key.
const CA: &[u8] = include_bytes!("claude_code/distribution-ca.pem");
const CERTIFICATE: &[u8] = include_bytes!("claude_code/distribution.pem");
const KEY: &[u8] = include_bytes!("claude_code/distribution.key");
/// Demi's shell tool, as the CLI names what it reaches over the SDK MCP
/// server `main`.
const SHELL: &str = "mcp__main__shell";
/// The variables that keep the CLI to the scripted vendor, besides its
/// address: the Cloud's own `PATH`, and the CLI's non-essential traffic,
/// telemetry and error reporting off. Its updater is the product's to turn
/// off (`crates/provider-claude-code/src/cli.rs`).
const QUIET: [(&str, &str); 4] = [
    (
        "PATH",
        "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
    ),
    ("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1"),
    ("DISABLE_TELEMETRY", "1"),
    ("DISABLE_ERROR_REPORTING", "1"),
];

/// The CLI the suite runs: its bytes, their SHA-256 and its version, read
/// once per process.
struct Supplied {
    bytes: Bytes,
    sha256: String,
    version: String,
}

static SUPPLIED: LazyLock<Supplied> = LazyLock::new(|| {
    let path = std::env::var_os("DEMI_TEST_CLAUDE_CODE")
        .expect("DEMI_TEST_CLAUDE_CODE names the Claude Code CLI");
    let bytes = Bytes::from(std::fs::read(&path).expect("the CLI can be read"));
    let sha256 = hex::encode(Sha256::digest(&bytes));
    Supplied {
        bytes,
        sha256,
        version: version_of(&path),
    }
});

/// The version the CLI prints, such as `2.1.283`, asked in a home of its
/// own and with the Cloud's quiet environment.
fn version_of(path: &std::ffi::OsStr) -> String {
    let home = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(path)
        .arg("--version")
        .env_clear()
        .envs(QUIET)
        .env("DISABLE_AUTOUPDATER", "1")
        .env("HOME", home.path())
        .env("CLAUDE_CONFIG_DIR", home.path())
        .output()
        .expect("the CLI starts");
    let printed = String::from_utf8_lossy(&output.stdout);
    let version = printed.split_whitespace().next().unwrap_or_default();
    assert!(
        demi_command_package_claude_code_protocol::is_version(version),
        "the CLI printed no version: {printed}"
    );
    version.to_owned()
}

/// The path of the suite's CA, which `SSL_CERT_FILE` must name: the backend
/// in this process reads the distribution with it, and the Cloud's runner
/// gets it for `demi.claude-code`.
fn trusted_ca() -> String {
    let named = std::env::var("SSL_CERT_FILE").expect(
        "SSL_CERT_FILE names claude_code/distribution-ca.pem",
    );
    let read = std::fs::read(&named).unwrap_or_default();
    assert_eq!(
        read, CA,
        "SSL_CERT_FILE={named} is not claude_code/distribution-ca.pem"
    );
    named
}

/// The distribution's manifest of the supplied CLI.
fn manifest(supplied: &Supplied) -> MockResponse {
    let build =
        json!({ "binary": "claude", "checksum": supplied.sha256, "size": supplied.bytes.len() });
    let platforms: serde_json::Map<String, Value> = PLATFORMS
        .iter()
        .map(|platform| ((*platform).to_owned(), build.clone()))
        .collect();
    let body = json!({ "version": supplied.version, "platforms": platforms });
    MockResponse::status(200)
        .header("content-type", "application/json")
        .chunk(body.to_string())
}

/// The Cloud runner's whole environment: the scripted vendor's address,
/// the quiet variables and the suite's CA, and nothing of this process's.
fn cloud_env(vendor: &MockVendor, ca: &str) -> BTreeMap<String, String> {
    let mut env: BTreeMap<String, String> = QUIET
        .iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect();
    env.insert("ANTHROPIC_BASE_URL".into(), vendor.url(""));
    env.insert("SSL_CERT_FILE".into(), ca.to_owned());
    env
}

/// A backend whose Claude Code machine installed the supplied CLI from the local
/// distribution, with a Claude Code entry whose account's token is made up,
/// and the scripted vendor the CLI infers from.
struct World {
    /// Held while the world lives: the scenarios' CLI processes keep their
    /// directories in the one `/tmp`, which a scenario counts.
    _serial: tokio::sync::MutexGuard<'static, ()>,
    harness: Harness,
    backend: TestBackend,
    master: Session,
    distribution: MockVendor,
    vendor: MockVendor,
    /// The Claude Code entry.
    provider: String,
}

impl World {
    /// Adding the account installs the CLI on the Claude Code machine; the world is ready
    /// once it is installed.
    async fn start() -> Self {
        static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
        let serial = SERIAL.lock().await;
        let supplied = &*SUPPLIED;
        let ca = trusted_ca();
        let distribution = MockVendor::start_tls(CERTIFICATE, KEY).await;
        let version = &supplied.version;
        distribution.respond_at(
            &format!("{RELEASES}/latest"),
            MockResponse::status(200).chunk(format!("{version}\n")),
        );
        distribution.respond_at(
            &format!("{RELEASES}/{version}/manifest.json"),
            manifest(supplied),
        );
        for platform in PLATFORMS {
            let executable = MockResponse::status(200).chunk(supplied.bytes.clone());
            distribution.respond_at(
                &format!("{RELEASES}/{version}/{platform}/claude"),
                executable,
            );
        }
        let vendor = MockVendor::start().await;
        vendor.respond_at("/api.json", claude_models());
        let harness = Harness::new()
            .with_claude_package()
            .with_claude_releases(distribution.url(RELEASES))
            .with_models_dev(vendor.url("/api.json"));
        harness
            .manager
            .script(|script| script.cloud_env = Some(cloud_env(&vendor, &ca)));
        let (backend, master) = harness.start_set_up().await;
        let secret = json!({
            "accessToken": TOKEN,
            "refreshToken": "sk-ant-ort01-demi-suite-made-up-refresh",
            "expiresAt": "2100-01-01T00:00:00.000Z",
            "scopes": ["user:inference", "user:profile"],
            "subscriptionType": "max",
            "accountId": "demi-suite-account",
            "email": "suite@example.test",
        });
        let provider = backend
            .seed_subscription(
                &master,
                "claude-code",
                "Claude",
                "account:demi-suite-account",
                secret.to_string(),
            )
            .await;
        // **Install**, as the settings page offers it.
        let started = backend
            .post(
                &format!("/api/providers/{provider}/cli/install"),
                Some(&master),
                json!({}),
            )
            .await;
        assert_eq!(started.status, StatusCode::ACCEPTED);
        let installed = settled(&backend, &master, &provider).await;
        assert!(
            matches!(installed, Some(CliInstall::Installed { .. })),
            "the Cloud did not install the CLI: {installed:?}"
        );
        Self {
            _serial: serial,
            harness,
            backend,
            master,
            distribution,
            vendor,
            provider,
        }
    }

    /// The Claude Code machine's home.
    fn home(&self) -> String {
        let machine = self.harness.manager.claude_code_machine();
        self.harness.manager.home(&machine)
    }

    /// The master's conversation, open on the Claude Code entry.
    async fn conversation(&self) -> Socket {
        create(&self.backend, &self.master, CONVERSATION).await;
        choose(
            &self.backend,
            &self.master,
            CONVERSATION,
            &self.provider,
            MODEL,
        )
        .await;
        let mut socket = Socket::connect(&self.backend, &self.master, CONVERSATION).await;
        socket.open().await;
        socket
    }

    /// The conversation's transcript as the backend serves it cold.
    async fn blocks(&self) -> Vec<Block> {
        transcript(&self.backend, &self.master, CONVERSATION)
            .await
            .blocks
    }

    /// Queues the vendor's answer to the next Messages API request.
    fn answers(&self, response: MockResponse) {
        self.vendor.respond_at("/v1/messages", response);
    }

    /// The Messages API requests the vendor received, in order.
    fn inferences(&self) -> Vec<RecordedRequest> {
        self.vendor
            .requests()
            .into_iter()
            .filter(|request| {
                request.method == Method::POST && request.uri.path() == "/v1/messages"
            })
            .collect()
    }
}

/// The CLI process a request came from: every process resumes the
/// conversation's session, but in a configuration directory of its own,
/// whose device id the CLI names in each request.
fn process(request: &RecordedRequest) -> String {
    let user: Value = serde_json::from_str(
        request.json()["metadata"]["user_id"]
            .as_str()
            .expect("the CLI names its user"),
    )
    .expect("the CLI's user is JSON");
    user["device_id"]
        .as_str()
        .expect("the CLI names its device")
        .to_owned()
}

/// The conversation's messages as a request carries them, without the
/// CLI's own `system` messages: each message's role and its texts.
fn messages(request: &Value) -> Vec<(String, Vec<String>)> {
    request["messages"]
        .as_array()
        .expect("the request has messages")
        .iter()
        .filter(|message| message["role"] != "system")
        .map(|message| {
            let texts = match &message["content"] {
                Value::String(text) => vec![text.clone()],
                blocks => blocks
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|block| block["text"].as_str().map(str::to_owned))
                    .collect(),
            };
            (
                message["role"].as_str().unwrap_or_default().to_owned(),
                texts,
            )
        })
        .collect()
}

/// A request's messages as the vendor caches them: without the CLI's cache
/// marks, which move to each request's end, and a text the CLI sends as a
/// string read as the one text block it is.
fn as_cached(request: &Value) -> Vec<Value> {
    fn unmarked(value: &Value) -> Value {
        match value {
            Value::Object(fields) => fields
                .iter()
                .filter(|(name, _)| *name != "cache_control")
                .map(|(name, value)| (name.clone(), unmarked(value)))
                .collect(),
            Value::Array(values) => values.iter().map(unmarked).collect(),
            value => value.clone(),
        }
    }
    request["messages"]
        .as_array()
        .expect("the request has messages")
        .iter()
        .map(|message| {
            let mut message = unmarked(message);
            if let Some(text) = message["content"].as_str() {
                message["content"] = json!([{ "type": "text", "text": text }]);
            }
            message
        })
        .collect()
}

/// Asserts that `later` begins with every message `earlier` sent, as the
/// vendor caches them.
fn begins_with(later: &Value, earlier: &Value) {
    let (earlier, later) = (as_cached(earlier), as_cached(later));
    assert!(
        later.len() > earlier.len(),
        "{} messages after {}: {later:#?}",
        later.len(),
        earlier.len(),
    );
    for (index, (sent, again)) in earlier.iter().zip(&later).enumerate() {
        assert_eq!(again, sent, "message {index} differs");
    }
}

/// Whether `frame` carries `text`, as the page reads it.
fn carries(frame: &ServerFrame, text: &str) -> bool {
    serde_json::to_string(frame).unwrap().contains(text)
}

// Several seconds, as in every scenario of the suite: the Cloud boots and
// installs `demi.claude-code`, which downloads and verifies the CLI (240 MB for
// 2.1.283), and each new CLI process takes about half a second to start.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CLAUDE_CODE naming the Claude Code CLI"]
async fn the_clouds_verified_cli_streams_reasoning_and_text_and_runs_a_tool_batch_through_demi_in_one_process()
 {
    let world = World::start().await;
    let supplied = &*SUPPLIED;

    // The install: the pointer, the manifest and this machine's build, each
    // read once, and an executable that is the supplied one, byte for byte.
    let install = settled(&world.backend, &world.master, &world.provider).await;
    // The CLI is an artifact of the package, in the runner's cache, where
    // it is the supplied one, byte for byte.
    let Some(CliInstall::Installed { path }) = &install else {
        panic!("the CLI is not installed: {install:?}");
    };
    assert_eq!(
        cli(&world.backend, &world.master, &world.provider).await.newest,
        NewestVersion::Read {
            version: supplied.version.clone()
        }
    );
    let installed = std::fs::read(&path).unwrap();
    assert_eq!(hex::encode(Sha256::digest(&installed)), supplied.sha256);
    let read: Vec<String> = world
        .distribution
        .requests()
        .iter()
        .map(|request| request.uri.path().to_owned())
        .collect();
    let [latest, manifest, build] = read.as_slice() else {
        panic!("the distribution was read {} times: {read:?}", read.len());
    };
    assert_eq!(latest, &format!("{RELEASES}/latest"));
    assert_eq!(
        manifest,
        &format!("{RELEASES}/{}/manifest.json", supplied.version)
    );
    assert!(
        build.starts_with(&format!("{RELEASES}/{}/", supplied.version)),
        "{build}"
    );

    // A first message: the CLI offers the model Demi's tools, over the SDK
    // MCP server its `initialize` declared, and streams the vendor's
    // reasoning and text.
    let mut socket = world.conversation().await;
    let usage_at_start = json!({
        "input_tokens": 11, "output_tokens": 1, "cache_read_input_tokens": 5, "cache_creation_input_tokens": 3,
    });
    world.answers(message(
        vec![
            thinking_block(0, "Weighing a greeting.", "signature-1"),
            text_block(1, &["Hello", " from the vendor."]),
        ],
        "end_turn",
        usage_at_start,
        7,
    ));
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a11", "Say hello.")
        .await;
    let blocks = world.blocks().await;
    // The conversation's first request tells the model where it runs.
    assert_eq!(
        kinds(&blocks),
        ["user", "context", "thinking", "text", "response"]
    );
    let Block::Thinking(thinking) = &blocks[2] else {
        unreachable!()
    };
    assert_eq!(
        (thinking.text.as_str(), thinking.signature.as_deref()),
        ("Weighing a greeting.", Some("signature-1"))
    );
    assert_eq!(last_text(&blocks), "Hello from the vendor.");
    let Block::Response(response) = &blocks[4] else {
        unreachable!()
    };
    let counted = TokenUsage {
        input_tokens: 11,
        output_tokens: 7,
        cache_read_tokens: 5,
        cache_write_tokens: 3,
    };
    assert_eq!(response.usage, Some(counted));
    let totals = usage(&world.backend, &world.master).await.totals;
    assert_eq!(totals.len(), 1, "{totals:?}");
    let row = &totals[0];
    assert_eq!(
        (
            row.requests,
            row.input_tokens,
            row.output_tokens,
            row.cache_read_tokens,
            row.cache_write_tokens
        ),
        (1, 11, 7, 5, 3)
    );
    let inferences = world.inferences();
    let [first] = inferences.as_slice() else {
        panic!("one request: {inferences:?}");
    };
    assert_eq!(
        first.header("authorization"),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    let body = first.json();
    assert_eq!(body["model"], MODEL);
    assert!(
        body["system"]
            .to_string()
            .contains("You are Demi"),
        "{}",
        body["system"]
    );
    let tools: Vec<&str> = body["tools"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|tool| tool["name"].as_str())
        .collect();
    assert!(tools.contains(&SHELL), "{tools:?}");
    // The message and the execution context the conversation's first
    // request tells, which the CLI sends as one user message.
    let sent = messages(&body);
    let [(role, texts)] = sent.as_slice() else {
        panic!("one message: {sent:?}");
    };
    assert_eq!(role, "user");
    assert_eq!(texts[0].trim_end(), "Say hello.");
    assert!(texts[1].starts_with("[Execution context"), "{texts:?}");

    // A batch of two tool calls in one message: the CLI asks Demi for each
    // over the SDK MCP channel, Demi runs both commands on the Cloud, and
    // the CLI sends both results to the vendor, all in the process that kept
    // the conversation.
    let call = |text: &str| json!({ "description": "Print the text", "script": format!("printf '{text}'") });
    let batch = vec![
        tool_use_block(
            0,
            "toolu_suite_1",
            SHELL,
            &call("the first ran on the Cloud"),
        ),
        tool_use_block(
            1,
            "toolu_suite_2",
            SHELL,
            &call("the second ran on the Cloud"),
        ),
    ];
    world.answers(message(
        batch,
        "tool_use",
        json!({ "input_tokens": 20, "output_tokens": 1 }),
        9,
    ));
    world.answers(message(
        vec![text_block(0, &["Both ran."])],
        "end_turn",
        json!({ "input_tokens": 30, "output_tokens": 1 }),
        4,
    ));
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a12", "Run the tools.")
        .await;
    let blocks = world.blocks().await;
    assert_eq!(
        kinds(&blocks[5..]),
        ["user", "tool_call", "tool_call", "response", "text", "response"],
        "{:?}",
        kinds(&blocks)
    );
    for (block, (id, printed)) in blocks[6..8].iter().zip([
        ("toolu_suite_1", "the first ran on the Cloud"),
        ("toolu_suite_2", "the second ran on the Cloud"),
    ]) {
        let Block::ToolCall(ran) = block else {
            unreachable!()
        };
        assert_eq!(
            (ran.tool_use_id.as_str(), ran.tool_name.as_str()),
            (id, "shell")
        );
        assert_eq!(ran.status, ToolCallStatus::Completed);
        let output: String = ran
            .output
            .iter()
            .filter_map(|block| match block {
                ToolResultContentBlock::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect();
        assert!(output.contains(printed), "{output}");
    }
    assert_eq!(last_text(&blocks), "Both ran.");
    // Each call to the vendor ends with a response carrying its own usage,
    // the context that call read: the batch's call when the batch comes
    // out, the answer's at the result, never the sum the CLI's result adds
    // up (`crates/provider-claude-code/src/output.rs`).
    let call_usage = |index: usize| {
        let Block::Response(response) = &blocks[index] else {
            panic!("no response at {index}: {:?}", kinds(&blocks))
        };
        let usage = response.usage.expect("the call reported its usage");
        (usage.input_tokens, usage.output_tokens)
    };
    assert_eq!(call_usage(8), (20, 9));
    assert_eq!(call_usage(10), (30, 4));
    // The ledger has a row's worth for each of the three calls.
    let totals = usage(&world.backend, &world.master).await.totals;
    assert_eq!(totals.len(), 1, "{totals:?}");
    let row = &totals[0];
    assert_eq!(
        (
            row.requests,
            row.input_tokens,
            row.output_tokens,
            row.cache_read_tokens,
            row.cache_write_tokens
        ),
        (3, 11 + 20 + 30, 7 + 9 + 4, 5, 3)
    );
    let inferences = world.inferences();
    let [first, called, answered] = inferences.as_slice() else {
        panic!("three requests: {inferences:?}");
    };
    assert_eq!(process(called), process(first));
    assert_eq!(process(answered), process(first));
    let continued = messages(&called.json());
    assert_eq!(
        continued.len(),
        3,
        "the kept process continued the conversation: {continued:?}"
    );
    assert_eq!(
        continued[2],
        ("user".to_owned(), vec!["Run the tools.".to_owned()])
    );
    let results: Vec<Value> = answered.json()["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|message| message["content"].as_array().cloned().unwrap_or_default())
        .filter(|block| block["type"] == "tool_result")
        .collect();
    let [first_result, second_result] = results.as_slice() else {
        panic!(
            "the vendor did not receive both results: {}",
            answered.json()
        );
    };
    assert_eq!(first_result["tool_use_id"], "toolu_suite_1");
    assert!(
        first_result
            .to_string()
            .contains("the first ran on the Cloud"),
        "{first_result}"
    );
    assert_eq!(second_result["tool_use_id"], "toolu_suite_2");
    assert!(
        second_result
            .to_string()
            .contains("the second ran on the Cloud"),
        "{second_result}"
    );
    drop(socket);
    world.backend.close().await;
}

// Several seconds: see the first scenario. It runs five CLI processes and a
// turn of another provider, since what it proves is what one process's
// session gives the next.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CLAUDE_CODE naming the Claude Code CLI"]
async fn each_new_process_resumes_the_session_the_blocks_hold_and_its_first_request_begins_with_the_last_one()
 {
    let world = World::start().await;
    let mut socket = world.conversation().await;
    let change = |body: Value| {
        let world = &world;
        async move {
            let changed = world
                .backend
                .patch(&format!("/api/conversations/{CONVERSATION}"), &world.master, body)
                .await;
            assert_eq!(
                changed.status,
                StatusCode::OK,
                "{}",
                String::from_utf8_lossy(&changed.body)
            );
        }
    };

    // A turn of reasoning and text, and a turn of a two-call batch, in one
    // process.
    world.answers(message(
        vec![
            thinking_block(0, "Weighing a greeting.", "signature-1"),
            text_block(1, &["Hello."]),
        ],
        "end_turn",
        json!({ "input_tokens": 10, "output_tokens": 1 }),
        3,
    ));
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a21", "Say hello.")
        .await;
    let call = |text: &str| json!({ "description": "Print the text", "script": format!("printf '{text}'") });
    world.answers(message(
        vec![
            tool_use_block(0, "toolu_suite_1", SHELL, &call("one")),
            tool_use_block(1, "toolu_suite_2", SHELL, &call("two")),
        ],
        "tool_use",
        json!({ "input_tokens": 20, "output_tokens": 1 }),
        5,
    ));
    world.answers(message(
        vec![text_block(0, &["Both ran."])],
        "end_turn",
        json!({ "input_tokens": 30, "output_tokens": 1 }),
        3,
    ));
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a22", "Run the tools.")
        .await;
    let first = world.inferences();
    let first_last = first.last().unwrap().json();

    // Another effort needs another process, which resumes the session the
    // blocks hold: its first request begins with the last request of the
    // process before it, history and all, so the vendor reads that part
    // from its cache.
    change(json!({ "thinkingEffort": "medium" })).await;
    world.answers(message(
        vec![text_block(0, &["With more effort."])],
        "end_turn",
        json!({ "input_tokens": 14, "output_tokens": 1 }),
        3,
    ));
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a23", "Once more.")
        .await;
    let inferences = world.inferences();
    let [.., effort] = inferences.as_slice() else {
        unreachable!()
    };
    assert_ne!(process(effort), process(&first[0]));
    let effort = effort.json();
    assert_eq!(effort["output_config"]["effort"], "medium");
    begins_with(&effort, &first_last);

    // Another model's process carries its own request's system prompt,
    // which names the model, not the one the session recorded.
    change(json!({ "model": { "providerId": world.provider, "modelId": OTHER_MODEL } })).await;
    world.answers(message(
        vec![text_block(0, &["Another model answers."])],
        "end_turn",
        json!({ "input_tokens": 15, "output_tokens": 1 }),
        3,
    ));
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a24", "Again, briefly.")
        .await;
    let inferences = world.inferences();
    let [.., other] = inferences.as_slice() else {
        unreachable!()
    };
    let other_process = process(other);
    let other = other.json();
    assert_eq!(other["model"], OTHER_MODEL);
    assert!(
        first_last["system"].to_string().contains(MODEL)
            && other["system"].to_string().contains(OTHER_MODEL),
        "the system prompt did not follow the model: {}",
        other["system"]
    );

    // A turn of another provider, then back to Claude Code: the next
    // process resumes that turn written in Claude's format, the call under
    // the name the CLI gives Demi's tool, after what the process before it
    // sent.
    let anthropic = anthropic(&world.backend, &world.master, &world.vendor).await;
    choose(
        &world.backend,
        &world.master,
        CONVERSATION,
        &anthropic,
        OTHER_MODEL,
    )
    .await;
    world.answers(message(
        vec![
            text_block(0, &["Looking."]),
            tool_use_block(1, "toolu_api_1", "shell", &call("three")),
        ],
        "tool_use",
        json!({ "input_tokens": 40, "output_tokens": 1 }),
        5,
    ));
    world.answers(message(
        vec![text_block(0, &["Seen."])],
        "end_turn",
        json!({ "input_tokens": 50, "output_tokens": 1 }),
        2,
    ));
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a25", "Look at it.")
        .await;
    choose(
        &world.backend,
        &world.master,
        CONVERSATION,
        &world.provider,
        OTHER_MODEL,
    )
    .await;
    world.answers(message(
        vec![text_block(0, &["Back."])],
        "end_turn",
        json!({ "input_tokens": 60, "output_tokens": 1 }),
        2,
    ));
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a26", "Back to you.")
        .await;
    let inferences = world.inferences();
    let [.., resumed] = inferences.as_slice() else {
        unreachable!()
    };
    assert_ne!(process(resumed), other_process);
    let resumed = resumed.json();
    begins_with(&resumed, &other);
    let contents: Vec<Value> = resumed["messages"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|message| message["content"].as_array().cloned().unwrap_or_default())
        .collect();
    let written_call = contents
        .iter()
        .find(|block| block["id"] == "toolu_api_1")
        .unwrap_or_else(|| panic!("the other provider's call is not in its place: {resumed}"));
    assert_eq!(written_call["name"], SHELL);
    assert!(
        contents.iter().any(|block| block["tool_use_id"] == "toolu_api_1"
            && block.to_string().contains("three")),
        "the other provider's result is not in its place: {resumed}"
    );
    let said: Vec<(String, Vec<String>)> = messages(&resumed)
        .into_iter()
        .map(|(role, texts)| {
            // The CLI's reminders the model reads beside a message.
            let texts = texts
                .into_iter()
                .filter(|text| !text.starts_with("<system-reminder>"))
                .collect();
            (role, texts)
        })
        .collect();
    let tail: Vec<(&str, Vec<&str>)> = said[said.len() - 5..]
        .iter()
        .map(|(role, texts)| (role.as_str(), texts.iter().map(String::as_str).collect()))
        .collect();
    assert_eq!(
        tail,
        [
            ("user", vec!["Look at it."]),
            ("assistant", vec!["Looking."]),
            ("user", vec![]),
            ("assistant", vec!["Seen."]),
            ("user", vec!["Back to you."]),
        ]
    );

    // Stop in the middle of a stream closes the process; the next one
    // resumes the text that reached the page.
    let mut streaming = vec![message_start(
        json!({ "input_tokens": 12, "output_tokens": 1 }),
    )];
    streaming.extend(text_block(0, &["The long answer begins"]));
    world.answers(MockResponse::event_stream(events(&streaming)).stay_open());
    socket
        .send(&send(
            "5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a27",
            "Write a long answer.",
        ))
        .await;
    socket
        .until(|frame| carries(frame, "The long answer begins"))
        .await;
    socket.stop().await;
    world.vendor.disconnected().await;
    assert_eq!(last_text(&world.blocks().await), "The long answer begins");
    world.answers(message(
        vec![text_block(0, &["A short answer."])],
        "end_turn",
        json!({ "input_tokens": 13, "output_tokens": 1 }),
        3,
    ));
    socket
        .chat(
            "5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a28",
            "Answer briefly instead.",
        )
        .await;
    assert_eq!(last_text(&world.blocks().await), "A short answer.");
    let inferences = world.inferences();
    let [.., stopped, after] = inferences.as_slice() else {
        unreachable!()
    };
    assert_ne!(process(after), process(stopped));
    let texts: Vec<String> = messages(&after.json())
        .into_iter()
        .flat_map(|(_, texts)| texts)
        .filter(|text| !text.starts_with("<system-reminder>"))
        .collect();
    assert_eq!(
        texts[texts.len() - 3..],
        [
            "Write a long answer.",
            "The long answer begins",
            "Answer briefly instead."
        ]
    );

    // No request carries the CLI's own account of its machine.
    let run_dir = format!("{}/run", world.home());
    for request in world.inferences() {
        let sent = request.json()["messages"].to_string();
        assert!(
            !sent.contains("Primary working directory")
                && !sent.contains("OS Version")
                && !sent.contains(&run_dir),
            "a request carries the CLI's environment: {sent}"
        );
    }
    drop(socket);
    world.backend.close().await;
}

// Several seconds: see the first scenario.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CLAUDE_CODE naming the Claude Code CLI"]
async fn a_vendor_error_fails_the_request_in_the_clis_words_and_the_kept_process_answers_the_next()
{
    let world = World::start().await;
    let mut socket = world.conversation().await;

    // The vendor refuses the request: the request fails with the CLI's words
    // for it, and the ledger has no row for it.
    // Claude Code 2.1.293 sends a refused call once more without its
    // thinking display before it gives up.
    let refusal = json!({ "type": "error", "error": {
        "type": "invalid_request_error", "message": "The scripted vendor refuses this request." } });
    for _ in 0..2 {
        world.answers(
            MockResponse::status(400)
                .header("content-type", "application/json")
                .chunk(refusal.to_string()),
        );
    }
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a31", "Hello?")
        .await;
    let blocks = world.blocks().await;
    assert_eq!(kinds(&blocks), ["user", "context", "error"]);
    let Block::Error(failed) = &blocks[2] else {
        unreachable!()
    };
    assert!(
        failed
            .message
            .contains("The scripted vendor refuses this request."),
        "{}",
        failed.message
    );
    assert!(usage(&world.backend, &world.master).await.totals.is_empty());

    // The next request goes on in the same process.
    world.answers(message(
        vec![text_block(0, &["Better now."])],
        "end_turn",
        json!({ "input_tokens": 8, "output_tokens": 1 }),
        2,
    ));
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a32", "Try again.")
        .await;
    let blocks = world.blocks().await;
    assert_eq!(
        kinds(&blocks[3..]),
        ["user", "text", "response"],
        "{:?}",
        kinds(&blocks)
    );
    assert_eq!(last_text(&blocks), "Better now.");
    let inferences = world.inferences();
    let [refused, _, answered] = inferences.as_slice() else {
        panic!("three requests: {inferences:?}");
    };
    assert_eq!(process(answered), process(refused));
    assert_eq!(
        usage(&world.backend, &world.master).await.totals[0].requests,
        1
    );
    drop(socket);
    world.backend.close().await;
}

// Several seconds: see the first scenario.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CLAUDE_CODE naming the Claude Code CLI"]
async fn a_call_of_a_tool_the_request_does_not_offer_is_answered_by_the_cli_and_the_turn_goes_on() {
    let world = World::start().await;
    let mut socket = world.conversation().await;

    // The model names a tool Demi does not offer: the CLI answers the call
    // with an error, the model corrects itself in the same process, and the
    // turn ends as any other (`crates/provider-claude-code/src/mcp.rs`).
    let unknown = "demi agent send parent";
    world.answers(message(
        vec![tool_use_block(0, "toolu_suite_unknown", unknown, &json!({}))],
        "tool_use",
        json!({ "input_tokens": 10, "output_tokens": 1 }),
        3,
    ));
    world.answers(message(
        vec![text_block(0, &["Corrected."])],
        "end_turn",
        json!({ "input_tokens": 12, "output_tokens": 1 }),
        2,
    ));
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a41", "Tell the lead.")
        .await;
    let blocks = world.blocks().await;
    assert_eq!(kinds(&blocks), ["user", "context", "text", "response"]);
    assert_eq!(last_text(&blocks), "Corrected.");
    let refused = |request: &RecordedRequest, id: &str| {
        let messages = request.json()["messages"].clone();
        let result = messages
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|message| message["content"].as_array().cloned().unwrap_or_default())
            .find(|block| block["type"] == "tool_result" && block["tool_use_id"] == id)
            .unwrap_or_else(|| panic!("no result for {id}: {messages}"));
        assert_eq!(result["is_error"], true, "{result}");
        assert!(
            result.to_string().contains("No such tool available"),
            "{result}"
        );
    };
    let inferences = world.inferences();
    let [called, corrected] = inferences.as_slice() else {
        panic!("two requests: {inferences:?}");
    };
    assert_eq!(process(corrected), process(called));
    refused(corrected, "toolu_suite_unknown");

    // In a batch with a call of Demi's tool, that call runs through Demi and
    // the other is the CLI's to answer.
    let call = json!({ "description": "Print the text", "script": "printf 'one'" });
    world.answers(message(
        vec![
            tool_use_block(0, "toolu_suite_shell", SHELL, &call),
            tool_use_block(1, "toolu_suite_ghost", unknown, &json!({})),
        ],
        "tool_use",
        json!({ "input_tokens": 20, "output_tokens": 1 }),
        5,
    ));
    world.answers(message(
        vec![text_block(0, &["One ran."])],
        "end_turn",
        json!({ "input_tokens": 30, "output_tokens": 1 }),
        3,
    ));
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a42", "Run it and tell the lead.")
        .await;
    let blocks = world.blocks().await;
    assert_eq!(
        kinds(&blocks[4..]),
        ["user", "tool_call", "response", "text", "response"],
        "{:?}",
        kinds(&blocks)
    );
    assert_eq!(last_text(&blocks), "One ran.");
    let inferences = world.inferences();
    let [.., answered] = inferences.as_slice() else {
        unreachable!()
    };
    assert_eq!(inferences.len(), 4, "{inferences:?}");
    refused(answered, "toolu_suite_ghost");
    let ran = answered.json()["messages"].to_string();
    assert!(ran.contains("toolu_suite_shell") && ran.contains("one"), "{ran}");
    drop(socket);
    world.backend.close().await;
}

// Several seconds: see the first scenario.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CLAUDE_CODE naming the Claude Code CLI"]
async fn the_cli_reads_the_sign_in_handed_to_its_accounts_directory_and_runs_in_a_directory_of_its_own() {
    let world = World::start().await;
    let mut socket = world.conversation().await;
    world.answers(message(
        vec![text_block(0, &["Signed in."])],
        "end_turn",
        json!({ "input_tokens": 5, "output_tokens": 1 }),
        2,
    ));
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a41", "Hello?")
        .await;
    assert_eq!(last_text(&world.blocks().await), "Signed in.");
    // The CLI called with the token the account's record held, which the
    // first start wrote into the account's directory as the CLI's own
    // sign-in; Demi handed the process none.
    let tokens: Vec<Option<String>> = world
        .inferences()
        .iter()
        .map(|request| request.header("authorization").map(str::to_owned))
        .collect();
    assert_eq!(tokens, [Some(format!("Bearer {TOKEN}"))]);
    let accounts = std::fs::read_dir(format!("{}/accounts", world.home()))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect::<Vec<_>>();
    let [account] = accounts.as_slice() else {
        panic!("one account's directory: {accounts:?}");
    };
    assert!(account.join(".credentials.json").exists());
    // The kept process's own directory lies under the account's, and goes
    // with it.
    let own: Vec<std::path::PathBuf> = std::fs::read_dir(account.join("projects"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    let [own] = own.as_slice() else {
        panic!("one process directory: {own:?}");
    };
    drop(socket);
    world.backend.close().await;
    assert!(!own.exists());
    assert!(account.join(".credentials.json").exists());
}

/// What the install left is kept for the Cloud's runner connection
/// (`crates/backend-providers/src/llm/claude_releases.rs`): with the Cloud's
/// `demi.claude-code` service stopped, so that the package answers nothing, a
/// conversation starts the installed CLI and its turn ends.
// Several seconds: see the first scenario.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CLAUDE_CODE naming the Claude Code CLI"]
async fn the_installed_cli_starts_while_the_package_answers_nothing() {
    let world = World::start().await;
    let mut service = PackageService::of(&world.harness);
    service.freeze();
    world.answers(message(
        vec![text_block(0, &["Hello."])],
        "end_turn",
        json!({ "input_tokens": 8, "output_tokens": 1 }),
        1,
    ));
    let mut socket = world.conversation().await;
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a41", "Hi")
        .await;
    assert_eq!(last_text(&world.blocks().await), "Hello.");
    drop(service);
    drop(socket);
    world.backend.close().await;
}

/// What the supplied CLI prints, started with the provider's flags that shape its
/// input and output, against a scripted vendor that answers `answers`, when
/// `messages` are written to it at once, each with a client id.
///
/// It returns each line's `type`, and `result:<indexes>` for a result, the
/// indexes in `messages` of the messages it names, until a result names the
/// last message.
async fn printed(messages: &[&str], answers: &[&str]) -> Vec<String> {
    use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _};

    let path = std::env::var_os("DEMI_TEST_CLAUDE_CODE")
        .expect("DEMI_TEST_CLAUDE_CODE names the Claude Code CLI");
    let vendor = MockVendor::start().await;
    for answer in answers {
        vendor.respond_at(
            "/v1/messages",
            message(
                vec![text_block(0, &[answer])],
                "end_turn",
                json!({ "input_tokens": 5, "output_tokens": 1 }),
                2,
            ),
        );
    }
    let home = tempfile::tempdir().unwrap();
    let mut cli = tokio::process::Command::new(path)
        .args([
            "--print",
            "--output-format",
            "stream-json",
            "--verbose",
            "--input-format",
            "stream-json",
            "--include-partial-messages",
            "--safe-mode",
            "--disable-slash-commands",
            "--tools",
            "",
            "--permission-mode",
            "bypassPermissions",
            "--allow-dangerously-skip-permissions",
            "--model",
            MODEL,
        ])
        .env_clear()
        .envs(QUIET)
        .env("DISABLE_AUTOUPDATER", "1")
        .env("HOME", home.path())
        .env("CLAUDE_CONFIG_DIR", home.path())
        .env("ANTHROPIC_BASE_URL", vendor.url(""))
        .env("ANTHROPIC_API_KEY", "sk-ant-api03-demi-suite-made-up-key")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("the CLI starts");
    let mut input = cli.stdin.take().unwrap();
    let ids: Vec<String> = messages.iter().map(|_| uuid::Uuid::new_v4().to_string()).collect();
    for (text, id) in messages.iter().zip(&ids) {
        let line = json!({ "type": "user", "message": { "role": "user", "content": text }, "uuid": id });
        input.write_all(format!("{line}\n").as_bytes()).await.unwrap();
    }
    input.flush().await.unwrap();

    let mut lines = tokio::io::BufReader::new(cli.stdout.take().unwrap()).lines();
    let mut printed: Vec<String> = Vec::new();
    let read = async {
        while let Some(line) = lines.next_line().await.unwrap() {
            let line: Value = serde_json::from_str(&line).unwrap();
            let named: Vec<usize> = line["user_message_uuids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|named| ids.iter().position(|id| Some(id.as_str()) == named.as_str()))
                .collect();
            let kind = match line["type"].as_str() {
                Some("result") => {
                    format!("result:{}", named.iter().map(usize::to_string).collect::<Vec<_>>().join(","))
                }
                Some("stream_event") => format!("stream_event:{}", line["event"]["type"].as_str().unwrap_or_default()),
                Some(kind) => kind.to_owned(),
                None => continue,
            };
            let done = kind.starts_with("result:") && named.contains(&(messages.len() - 1));
            printed.push(kind);
            if done {
                return;
            }
        }
        panic!("the CLI's output ended: {printed:?}");
    };
    tokio::time::timeout(std::time::Duration::from_secs(60), read)
        .await
        .unwrap_or_else(|_| panic!("a result named the last message within a minute: {printed:?}"));
    drop(input);
    cli.kill().await.unwrap();
    printed
}

/// Each `result` names the client ids of the user messages its turn consumed,
/// all of a batch the CLI merged into one turn among them: a run ends at the
/// `result` that names the last message it wrote
/// (`crates/provider-claude-code/src/run.rs`). Run it for each new version of
/// the CLI before Demi installs it.
///
/// Three messages are written at once, as a run writes three subagents'
/// receipts. However the CLI groups them into turns, its results name each
/// message once, in order.
// One process of the supplied CLI and up to three answers of the scripted
// vendor: a few seconds, most of it the CLI's start.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CLAUDE_CODE naming the Claude Code CLI"]
async fn each_result_names_every_message_its_turn_consumed() {
    let written = ["First receipt.", "Second receipt.", "Third receipt."];
    let printed = printed(&written, &["First answer.", "Second answer.", "Third answer."]).await;
    let named: Vec<&str> = printed
        .iter()
        .filter_map(|kind| kind.strip_prefix("result:"))
        .flat_map(|names| names.split(',').filter(|name| !name.is_empty()))
        .collect();
    assert_eq!(named, ["0", "1", "2"], "{printed:?}");
}

/// A message that starts with `/`, which the CLI takes for a command it does
/// not offer, is answered by the CLI itself: it prints an answer and a `result`
/// with no model call, which names the message, so it ends the run
/// (`crates/provider-claude-code/src/run.rs`).
///
/// A path such as `/Users/…` is a message like any other.
// Two processes of the supplied CLI and one answer of the scripted vendor: a
// few seconds.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CLAUDE_CODE naming the Claude Code CLI"]
async fn a_command_the_cli_does_not_offer_gets_a_result_that_names_it_with_no_model_call() {
    let printed_for_command = printed(&["/cost"], &[]).await;
    assert!(
        !printed_for_command.iter().any(|kind| kind.starts_with("stream_event:")),
        "{printed_for_command:?}"
    );
    assert_eq!(printed_for_command.last().map(String::as_str), Some("result:0"));

    let path = "/Users/x/a.txt 是什么";
    let printed_for_path = printed(&[path], &["A text file."]).await;
    assert!(printed_for_path.contains(&"stream_event:message_start".to_owned()), "{printed_for_path:?}");
    assert_eq!(printed_for_path.last().map(String::as_str), Some("result:0"));
}
