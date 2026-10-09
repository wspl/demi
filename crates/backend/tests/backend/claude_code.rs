//! The real Claude Code CLI through the backend (`scenarios.md` § Real
//! machine acceptance, `claude-code.md`): the user's Cloud installs Demi's
//! copy of the CLI from a local distribution and verifies it, and a
//! conversation infers through it as in the product. The CLI declares Demi's
//! SDK MCP server and offers the model its tools, streams reasoning and
//! text, runs a batch of tool calls through Demi and sends back their
//! results, reports usage, stops in the middle of a stream, and reports a
//! vendor error as the request's failure. Each new process, for another
//! model and effort, after another provider's turn or after a stop,
//! resumes the session the blocks hold: its first request begins with the
//! last request of the process before it, with no account of the CLI's
//! machine. It reads the account's access
//! token from a descriptor and runs in a private directory of its own that
//! goes with it; a turn the vendor refuses for its token is refreshed at the
//! token endpoint and sent again in a new process. Every assertion is on what the product
//! observes: the transcript, the frames, the usage ledger and what the
//! vendor received.
//!
//! The suite runs only when `DEMI_TEST_CLAUDE_CODE` names the CLI's
//! executable, as an ordinary user, since the CLI refuses the provider's
//! permission mode as root, with `SSL_CERT_FILE` naming
//! `claude_code/distribution-ca.pem` (`builds-and-releases.md` §
//! Validation). Nothing reaches the vendor's service: the CLI's inference
//! goes to the scripted vendor on 127.0.0.1, which the Cloud's runner names
//! in `ANTHROPIC_BASE_URL`; the runner's environment is the suite's own, so
//! no proxy of the machine's reaches the CLI; the CLI's non-essential
//! traffic, telemetry and error reporting are off; and the account's tokens
//! are made up. The account is stored as a sign-in would store it, since
//! the CLI's own sign-in reaches only the vendor.
//!
//! The distribution serves HTTPS, since `demi.claude-code` downloads nothing
//! else, with the certificate in `claude_code/`. The backend in this process
//! and the Cloud's `demi.claude-code` trust its CA through `SSL_CERT_FILE`, which
//! the machine's roots of the one selection's reqwest read.

use std::collections::BTreeMap;
use std::sync::LazyLock;

use bytes::Bytes;
use demi_conversation_socket_protocol::ServerFrame;
use demi_provider_common::testing::{MockResponse, MockVendor, RecordedRequest};
use demi_shared_types::{Block, TokenUsage, ToolCallStatus, ToolResultContentBlock};
use demi_backend::families::ClaudeCodeFamily;
use demi_web_api_protocol::providers::{CliInstall, NewestVersion};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use crate::claude::{claude_models, settled};
use crate::conversations::{
    Socket, anthropic, choose, create, events, kinds, last_text, message, message_start, send,
    text_block, thinking_block, tool_use_block, transcript, usage,
};
use crate::support::{Harness, Session, TestBackend};

/// The account's access token: made up, since no request reaches the
/// vendor.
const TOKEN: &str = "sk-ant-oat01-demi-suite-made-up-token";
/// The access token a refresh gives.
const FRESH: &str = "sk-ant-oat01-demi-suite-fresh-token";
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
const SHELL: &str = "mcp__main__shell_exec";
/// The variables that keep the CLI to the scripted vendor, besides its
/// address: the Cloud's own `PATH`, and the CLI's non-essential traffic,
/// telemetry and error reporting off. Its updater is the product's to turn
/// off (`claude-code.md` § Starting).
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
        "SSL_CERT_FILE names claude_code/distribution-ca.pem (builds-and-releases.md § Validation)",
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

/// A backend whose user's Cloud installed the supplied CLI from the local
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
    /// Adding the account installs the CLI on the Cloud; the world is ready
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
        // The account's tokens are refreshed at the scripted vendor.
        let claude = ClaudeCodeFamily {
            token_url: vendor.url("/v1/oauth/token").parse().unwrap(),
            ..ClaudeCodeFamily::default()
        };
        let harness = Harness::new()
            .with_claude_package()
            .with_claude_releases(distribution.url(RELEASES))
            .with_models_dev(vendor.url("/api.json"))
            .with_families(demi_backend::families::builtin().with("claude-code", claude));
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
            matches!(installed.install, Some(CliInstall::Installed { .. })),
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

    /// The user's Cloud's home.
    fn home(&self) -> String {
        let devices = self.harness.manager.devices();
        let [cloud] = devices.as_slice() else {
            panic!("the user has one Cloud: {devices:?}");
        };
        self.harness.manager.home(cloud)
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
#[ignore = "requires DEMI_TEST_CLAUDE_CODE naming the Claude Code CLI, as builds-and-releases.md § Validation runs it"]
async fn the_clouds_verified_cli_streams_reasoning_and_text_and_runs_a_tool_batch_through_demi_in_one_process()
 {
    let world = World::start().await;
    let supplied = &*SUPPLIED;

    // The install: the pointer, the manifest and this machine's build, each
    // read once, and an executable that is the supplied one, byte for byte.
    let cli = settled(&world.backend, &world.master, &world.provider).await;
    // The CLI is an artifact of the package, in the runner's cache, where
    // it is the supplied one, byte for byte.
    let Some(CliInstall::Installed { path }) = &cli.install else {
        panic!("the CLI is not installed: {cli:?}");
    };
    assert_eq!(
        cli.newest,
        NewestVersion::Read {
            version: supplied.version.clone()
        }
    );
    assert_eq!(
        cli.machines[0].versions,
        Some(vec![supplied.version.clone()])
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
    assert_eq!(response.usage, counted);
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
    let call = |text: &str| json!({ "description": "Print the text", "script": format!("printf '{text}'"), "timeoutMs": 60_000 });
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
        ["user", "tool_call", "tool_call", "text", "response"],
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
            (id, "shell_exec")
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
    // The CLI's result reports the turn's two calls together and lists no
    // calls one by one, so the response carries their sum
    // (`claude-code.md` § Reading).
    let Block::Response(response) = &blocks[9] else {
        unreachable!()
    };
    assert_eq!(
        (response.usage.input_tokens, response.usage.output_tokens),
        (50, 13)
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
#[ignore = "requires DEMI_TEST_CLAUDE_CODE naming the Claude Code CLI, as builds-and-releases.md § Validation runs it"]
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
    let call = |text: &str| json!({ "description": "Print the text", "script": format!("printf '{text}'"), "timeoutMs": 60_000 });
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
            tool_use_block(1, "toolu_api_1", "shell_exec", &call("three")),
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
    let run_dir = format!("{}/.demi/claude/run", world.home());
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
#[ignore = "requires DEMI_TEST_CLAUDE_CODE naming the Claude Code CLI, as builds-and-releases.md § Validation runs it"]
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

/// The configuration directories of CLI processes on this machine.
fn config_directories() -> std::collections::BTreeSet<std::path::PathBuf> {
    std::fs::read_dir("/tmp")
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with("demi-claude-"))
        })
        .collect()
}

// Several seconds: see the first scenario.
#[tokio::test]
#[ignore = "requires DEMI_TEST_CLAUDE_CODE naming the Claude Code CLI, as builds-and-releases.md § Validation runs it"]
async fn the_cli_reads_its_token_from_a_descriptor_in_its_own_directory_and_a_refused_turn_runs_again_with_a_fresh_one()
 {
    use std::os::unix::fs::PermissionsExt as _;

    let world = World::start().await;
    let before = config_directories();
    let mut socket = world.conversation().await;

    // The vendor refuses the token the CLI read from its descriptor as
    // expired, on both of the CLI's own attempts (Claude Code 2.1.294 tries
    // twice). Demi refreshes the token at the token endpoint with the
    // account's refresh token, closes the process and sends the turn again
    // in a new one, which calls with the fresh token.
    let expired = json!({ "type": "error", "error": {
        "type": "authentication_error", "message": "OAuth token has expired" } });
    for _ in 0..2 {
        world.answers(
            MockResponse::status(401)
                .header("content-type", "application/json")
                .chunk(expired.to_string()),
        );
    }
    world.vendor.respond_at(
        "/v1/oauth/token",
        MockResponse::status(200)
            .header("content-type", "application/json")
            .chunk(
                json!({
                    "access_token": FRESH,
                    "refresh_token": "sk-ant-ort01-demi-suite-next-refresh",
                    "expires_in": 28_800,
                    "scope": "user:inference user:profile",
                })
                .to_string(),
            ),
    );
    world.answers(message(
        vec![text_block(0, &["Fresh again."])],
        "end_turn",
        json!({ "input_tokens": 5, "output_tokens": 1 }),
        2,
    ));
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a41", "Hello?")
        .await;
    // The user sees the turn once.
    let blocks = world.blocks().await;
    assert_eq!(kinds(&blocks), ["user", "context", "text", "response"]);
    assert_eq!(last_text(&blocks), "Fresh again.");
    let inferences = world.inferences();
    let tokens: Vec<Option<String>> = inferences
        .iter()
        .map(|request| request.header("authorization").map(str::to_owned))
        .collect();
    let stale = Some(format!("Bearer {TOKEN}"));
    assert_eq!(
        tokens,
        [stale.clone(), stale, Some(format!("Bearer {FRESH}"))]
    );
    assert_ne!(process(&inferences[2]), process(&inferences[0]));
    let refreshes: Vec<Value> = world
        .vendor
        .requests()
        .iter()
        .filter(|request| request.uri.path() == "/v1/oauth/token")
        .map(RecordedRequest::json)
        .collect();
    assert_eq!(
        refreshes,
        [json!({
            "grant_type": "refresh_token",
            "refresh_token": "sk-ant-ort01-demi-suite-made-up-refresh",
            "client_id": "9d1c250a-e61b-44d9-88ed-5944d1962f5e",
            "scope": "user:inference user:profile",
        })]
    );

    // The kept process runs in a directory of its own, private to the
    // Cloud's user, which goes with it; the refused process's went with it.
    let created: Vec<std::path::PathBuf> = config_directories()
        .difference(&before)
        .cloned()
        .collect();
    let [directory] = created.as_slice() else {
        panic!("one process directory: {created:?}");
    };
    let mode = std::fs::metadata(directory).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
    drop(socket);
    world.backend.close().await;
    assert!(!directory.exists());
}
