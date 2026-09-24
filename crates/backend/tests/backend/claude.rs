//! The Claude Code CLI through the backend (`claude-code.md` § Where it
//! runs, § What the user sees, § Acceptance): adding an account installs the
//! CLI on the user's Cloud, and an install that failed says why, with the
//! version, and fails a request the same way; the settings read the newest
//! version and the Cloud's versions; a Cloud with an older CLI answers with
//! it; a conversation on a paired device infers through the Cloud, whose
//! CLI process has the account's token; and after another account is
//! selected, the next request closes that process and starts one with the
//! new account's token.
//!
//! The distribution is a server the test scripts, and `demi.claude` is the
//! workspace's own, which downloads only over HTTPS: every install from the
//! scripted distribution fails. The Cloud's CLI is therefore a script the
//! test puts in its home with a receipt, where `claude.ensure` installs; it
//! answers each message with where it runs and with which token. No test
//! runs the real CLI.

use std::os::unix::fs::PermissionsExt as _;
use std::time::Duration;

use demi_agent::testing::model_of;
use demi_core::Block;
use demi_provider::testing::{MockResponse, MockVendor};
use demi_web_api::providers::{AddedAccount, CliInstall, NewestVersion, ProviderAnswer, ProviderCli};
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::conversations::{Socket, create, last_text, on_device, transcript};
use crate::conversations::settled as conversation_settled;
use crate::support::{Harness, Session, TestBackend};

const CONVERSATION: &str = "3c1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01";
/// The version the distribution names as its newest.
const NEWEST: &str = "2.1.3";
/// The version the Cloud has.
const OLDER: &str = "2.1.2";
const FIRST_TOKEN: &str = "sk-ant-oat01-first-account";
const SECOND_TOKEN: &str = "sk-ant-oat01-second-account";

/// The Claude Code CLI as a script: it answers `initialize`, answers each
/// user message with the executable, the token, the directory it runs in
/// and its configuration home, and records its start and its end.
const SCRIPTED_CLI: &str = r#"#!/bin/sh
log="$HOME/claude-processes.log"
echo "started $CLAUDE_CODE_OAUTH_TOKEN" >> "$log"
trap 'echo "ended $CLAUDE_CODE_OAUTH_TOKEN" >> "$log"; exit 0' TERM
while IFS= read -r line; do
  case "$line" in
    '{"type":"control_request"'*'"subtype":"initialize"'*)
      id=$(printf '%s\n' "$line" | sed 's/^{"type":"control_request","request_id":"\([^"]*\)".*/\1/')
      printf '{"type":"control_response","response":{"subtype":"success","request_id":"%s"}}\n' "$id"
      ;;
    '{"type":"user"'*)
      printf '{"type":"assistant","message":{"content":[{"type":"text","text":"cli=%s token=%s cwd=%s config=%s"}]}}\n' \
        "$0" "$CLAUDE_CODE_OAUTH_TOKEN" "$(pwd)" "$CLAUDE_CONFIG_DIR"
      printf '{"type":"result","usage":{"input_tokens":3,"output_tokens":2}}\n'
      ;;
  esac
done
"#;

/// The distribution's manifest of `version`.
fn manifest(version: &str) -> MockResponse {
    let build = json!({ "binary": "claude", "checksum": "ab".repeat(32), "size": 1024 });
    let body = json!({
        "version": version,
        "platforms": { "linux-x64": build, "linux-arm64": build, "darwin-arm64": build },
    });
    MockResponse::status(200)
        .header("content-type", "application/json")
        .chunk(body.to_string())
}

/// Puts `version` of the scripted CLI in the Cloud's home, with the receipt
/// an install leaves, and answers the executable's path.
fn install_by_hand(home: &str, version: &str) -> String {
    let directory = format!("{home}/.demi/claude/{version}");
    std::fs::create_dir_all(&directory).unwrap();
    let executable = format!("{directory}/claude");
    std::fs::write(&executable, SCRIPTED_CLI).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let receipt = json!({
        "version": version,
        "platform": "linux-x64",
        "sha256": "ab".repeat(32),
        "size": SCRIPTED_CLI.len(),
    });
    std::fs::write(format!("{directory}/receipt.json"), receipt.to_string()).unwrap();
    executable
}

async fn cli(backend: &TestBackend, session: &Session, provider: &str) -> ProviderCli {
    let read = backend.get(&format!("/api/providers/{provider}/cli"), Some(session)).await;
    assert_eq!(read.status, StatusCode::OK, "{}", String::from_utf8_lossy(&read.body));
    read.json()
}

/// The entry's CLI once no install is under way, asking every 50 ms for at
/// most 30 s.
async fn settled(backend: &TestBackend, session: &Session, provider: &str) -> ProviderCli {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let read = cli(backend, session, provider).await;
        if read.install != Some(CliInstall::Installing {}) {
            return read;
        }
        assert!(tokio::time::Instant::now() < deadline, "the install never ended");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The conversation's transcript once its turn is saved.
async fn saved(backend: &TestBackend, session: &Session) -> Vec<Block> {
    conversation_settled(backend, session, CONVERSATION).await;
    transcript(backend, session, CONVERSATION).await.blocks
}

#[tokio::test]
async fn a_conversation_on_a_paired_device_infers_through_the_clouds_cli_with_the_active_accounts_token() {
    let distribution = MockVendor::start().await;
    distribution.respond_at("/releases/latest", MockResponse::status(200).chunk(format!("{NEWEST}\n")));
    distribution.respond_at(&format!("/releases/{NEWEST}/manifest.json"), manifest(NEWEST));
    let harness = Harness::new()
        .with_claude_package()
        .with_claude_releases(distribution.url("/releases"));
    let (backend, master) = harness.start_set_up().await;

    // Adding the first account starts the install on the Cloud, which has no
    // CLI; the install fails, and the account stays.
    let imported = backend
        .post(
            "/api/providers/setup-token",
            Some(&master),
            json!({ "token": FIRST_TOKEN, "label": "Claude" }),
        )
        .await;
    assert_eq!(imported.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&imported.body));
    let provider = imported.json::<ProviderAnswer>().provider.id.as_str().to_owned();
    let failed = settled(&backend, &master, &provider).await;
    let Some(CliInstall::Failed { message }) = &failed.install else {
        panic!("the install did not fail: {failed:?}");
    };
    let reason = format!("Claude Code {NEWEST} could not be installed: invalid release record");
    assert!(message.starts_with(&reason), "{message}");
    assert_eq!(failed.newest, NewestVersion::Read { version: NEWEST.into() });
    let devices = harness.manager.devices();
    assert_eq!(devices.len(), 1, "the install woke the user's Cloud: {devices:?}");
    let cloud = &devices[0];
    let machines: Vec<(&str, Option<Vec<String>>)> = failed
        .machines
        .iter()
        .map(|machine| (machine.device_id.as_str(), machine.versions.clone()))
        .collect();
    assert_eq!(machines, [(cloud.as_str(), Some(Vec::new()))]);
    let accounts: Value = backend.get(&format!("/api/providers/{provider}/accounts"), Some(&master)).await.json();
    assert_eq!(accounts["accounts"].as_array().map(Vec::len), Some(1));

    // A request of a conversation whose files are on a paired laptop needs
    // the CLI on the Cloud: it fails the same way.
    create(&backend, &master, CONVERSATION).await;
    let (laptop, _) = on_device(&harness, &backend, &master, CONVERSATION).await;
    let model = model_of(&provider, "claude-opus-4-8");
    let mut socket = Socket::connect(&backend, &master, CONVERSATION).await;
    socket.open(&model).await;
    socket.chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01", "hello").await;
    let Some(Block::Error(error)) = saved(&backend, &master).await.pop() else {
        panic!("the request did not fail");
    };
    assert!(error.message.starts_with(&reason), "{}", error.message);

    // A Cloud with an older CLI answers with it; **Install** says so.
    let home = harness.manager.home(cloud);
    let older = install_by_hand(&home, OLDER);
    let started = backend
        .post(&format!("/api/providers/{provider}/cli/install"), Some(&master), json!({}))
        .await;
    assert_eq!(started.status, StatusCode::ACCEPTED, "{}", String::from_utf8_lossy(&started.body));
    let installed = settled(&backend, &master, &provider).await;
    assert_eq!(installed.install, Some(CliInstall::Installed { path: older.clone() }));
    assert_eq!(installed.machines[0].versions, Some(vec![OLDER.to_owned()]));
    // The distribution was read once: its answer is believed.
    assert_eq!(distribution.requests().len(), 2);

    // The request infers through the Cloud's CLI, in Demi's directories
    // there, with the account's token; nothing of it reaches the laptop.
    socket.chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a02", "hello again").await;
    let answer = |token: &str| {
        format!("cli={older} token={token} cwd={home}/.demi/claude/run config={home}/.demi/claude/config")
    };
    assert_eq!(last_text(&saved(&backend, &master).await), answer(FIRST_TOKEN));
    assert!(!laptop.runner.home_dir().join(".demi/claude").exists());

    // After another account is selected, the next request closes the
    // process and starts one with that account's token.
    let added = backend
        .post(&format!("/api/providers/{provider}/accounts"), Some(&master), json!({ "token": SECOND_TOKEN }))
        .await;
    assert_eq!(added.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&added.body));
    let second = added.json::<AddedAccount>().account.id;
    let selected = backend
        .put(
            &format!("/api/providers/{provider}/accounts/active"),
            &master,
            json!({ "credentialId": second }),
        )
        .await;
    assert_eq!(selected.status, StatusCode::OK, "{}", String::from_utf8_lossy(&selected.body));
    socket.chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a03", "and again").await;
    assert_eq!(last_text(&saved(&backend, &master).await), answer(SECOND_TOKEN));
    let processes = std::fs::read_to_string(format!("{home}/claude-processes.log")).unwrap();
    let expected = [
        format!("started {FIRST_TOKEN}"),
        format!("ended {FIRST_TOKEN}"),
        format!("started {SECOND_TOKEN}"),
    ];
    assert_eq!(processes.lines().collect::<Vec<_>>(), expected);
    backend.close().await;
}
