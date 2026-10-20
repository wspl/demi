//! The Claude Code CLI through the backend
//! (`crates/provider-claude-code/src/account.rs`,
//! `crates/provider-claude-code/src/placement.rs`,
//! `crates/backend-user-shard/src/conversation/claude_cli.rs`): a sign-in
//! runs the CLI's own login on the user's Cloud, relays its link, hands it
//! the pasted code, shows the CLI's words for a code it refuses, and keeps
//! the account it wrote, replacing the tokens of one signed in again, in a
//! directory that goes however the sign-in ends; a sign-in whose Cloud cannot install the CLI
//! says why, with the version, and so does a request; the settings read the
//! newest version and the Cloud's versions; a Cloud with an older CLI
//! answers with it; a Cloud with a CLI starts it without waiting for the
//! release pointer, whose answer a read in the background keeps, while a
//! Cloud with none waits for it; a session describes the Cloud as its
//! runner's hello reported it, with no process run for it; a
//! conversation on a paired device infers through the
//! Cloud, whose CLI process reads the account's token from a descriptor in
//! a private directory of its own; and after another account is selected,
//! the next request closes that process, whose directory goes, and starts
//! one with the new account's token in a new directory.
//!
//! The distribution is a server the test scripts, and `demi.claude-code` is the
//! workspace's own, which takes HTTPS downloads only: every install from the
//! scripted distribution fails. The Cloud's CLI is therefore a script the
//! test installs in the Cloud runner's artifact cache, as the runner installs
//! a download. Its login prints a link, reads a code, and writes the account
//! the code names as the CLI writes it; its other runs answer each message
//! with where they run and with which token. No test runs the real CLI.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt as _;
use std::time::Duration;

use demi_command_protocol::{ArtifactForm, PackageArtifact};
use demi_runner_command_packages::cache::{ArtifactCache, SILENT, Wanted};
use demi_runner_command_packages::{ArtifactResolver, ArtifactSource, RuntimeError};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use demi_provider_common::testing::{MockResponse, MockVendor};
use demi_shared_types::{Block, LoginStage};
use demi_web_api_protocol::diagnostics::{ServerLog, ServerLogLine};
use demi_web_api_protocol::state::SyncEvent;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::providers::{
    Accounts, CliInstall, CodeTaken, LoginStarted, LoginState, NewestVersion, ProviderCli, Providers,
};
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::accounts::{awaited, ended, followed, login_state, start_login};
use crate::diagnostics::{lines_of, request_id};
use crate::conversations::{Socket, choose, create, last_text, on_device, send, transcript};
use crate::support::{Harness, PATIENCE, ServerLogCapture, Session, TestBackend, eventually};

const CONVERSATION: &str = "3c1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01";
/// The version the distribution names as its newest.
const NEWEST: &str = "2.1.3";
/// The version the Cloud has.
const OLDER: &str = "2.1.2";
/// The link the CLI's login prints.
const LINK: &str = "https://claude.com/cai/oauth/authorize?code=true&state=scripted";
const LOGIN: &str = "/api/providers/subscription-login";

/// The Claude Code CLI as a script. Its login records its directory and that
/// directory's mode, prints the link and waits for a code: a code starting
/// `invalid` it calls invalid and waits again, one starting `refused` fails
/// the login, and any other, `account#state`, signs in `account` with the
/// access token `access-account-state`, whose tokens and account it writes
/// into its directory as the CLI writes them; the vendor refuses the sign-in
/// `access-expired-state`, as after a renewal it refused. Its other runs read the
/// access token from the sign-in in their configuration directory, answer
/// the message the session they resume ends with, if it ends with one, and
/// otherwise wait for input as a process prepared ahead does, answer
/// `initialize` and `get_usage`, print each user message back as they take
/// it and answer it with the
/// executable, the token, how many environment variables hold it, whether
/// descriptor 3 is open, the directory they run in, their configuration
/// directory and its mode and their own directory, and record the start
/// and the end of each process that resumes a session. While
/// `$HOME/hold-term` exists, a process asked to end writes its process id
/// to `$HOME/term-held` and ends only once `$HOME/release-term` appears. A session or message that asks for `three calls`
/// is answered by three calls to the vendor in one run, each streamed with
/// its message's id and its own counts, then the quota the vendor reported
/// after the last one, as the CLI prints a `rate_limit_event` after a call's
/// `message_stop`, and a `result` whose usage sums them.
const SCRIPTED_CLI: &str = r#"#!/bin/sh
if [ "$1" = auth ]; then
  echo "login $CLAUDE_CONFIG_DIR $(ls -ld "$CLAUDE_CONFIG_DIR" | cut -c1-10)" >> "$HOME/claude-logins.log"
  echo "Opening browser to sign in…"
  echo "If the browser didn't open, visit: https://claude.com/cai/oauth/authorize?code=true&state=scripted"
  printf 'Paste code here if prompted > '
  while IFS= read -r code; do
    case "$code" in
      invalid*) echo "Invalid code. Please make sure the full code was copied." >&2 ;;
      refused*) echo "Login failed: the code was refused" >&2; exit 1 ;;
      *) break ;;
    esac
  done
  account="${code%%#*}"
  state="${code#*#}"
  printf '{"claudeAiOauth":{"accessToken":"access-%s-%s","refreshToken":"refresh-%s","expiresAt":4102444800000,"scopes":["user:inference","user:profile"],"subscriptionType":"max","rateLimitTier":null}}' "$account" "$state" "$account" > "$CLAUDE_CONFIG_DIR/.credentials.json"
  printf '{"numStartups":1,"oauthAccount":{"accountUuid":"uuid-%s","emailAddress":"%s@example.test","organizationUuid":"org-1"}}' "$account" "$account" > "$CLAUDE_CONFIG_DIR/.claude.json"
  echo "Login successful."
  exit 0
fi
log="$HOME/claude-processes.log"
token=$(sed -n 's/.*"accessToken":"\([^"]*\)".*/\1/p' "$CLAUDE_CONFIG_DIR/.credentials.json")
own="$CLAUDE_CONFIG_DIR/projects/$CLAUDE_CODE_PROJECT_DIR_NAME"
session=$(ls "$own"/*.jsonl 2>/dev/null | head -n 1)
# A process that resumes a session records its start and its end; a usage
# probe resumes none.
if [ -n "$session" ]; then
  echo "started $token $own" >> "$log"
  trap 'if [ -e "$HOME/hold-term" ]; then echo $$ > "$HOME/term-held"; while [ ! -e "$HOME/release-term" ]; do sleep 0.02; done; rm -f "$HOME/term-held" "$HOME/release-term"; fi; echo "ended $token" >> "$log"; exit 0' TERM
fi
calls() {
  for n in 1 2 3; do
    printf '{"type":"stream_event","event":{"type":"message_start","message":{"id":"msg_call_%s","usage":{"input_tokens":1%s,"cache_read_input_tokens":%s00,"cache_creation_input_tokens":%s0,"cache_creation":{"ephemeral_1h_input_tokens":%s}}}}}\n' "$n" "$n" "$n" "$n" "$n"
    printf '{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}}\n'
    printf '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"call %s. "}}}\n' "$n"
    printf '{"type":"stream_event","event":{"type":"content_block_stop","index":0}}\n'
    printf '{"type":"stream_event","event":{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":%s}}}\n' "$n"
    printf '{"type":"stream_event","event":{"type":"message_stop"}}\n'
  done
  # As Claude Code 2.1 prints it, after the call's message_stop.
  printf '{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1791882600,"rateLimitType":"five_hour","overageStatus":"rejected","overageDisabledReason":"org_level_disabled","isUsingOverage":false,"unifiedWindows":{"five_hour":{"utilization":0.31,"resetsAt":1791882600},"seven_day":{"utilization":0.05,"resetsAt":1792447200}}}}\n'
  printf '{"type":"result","usage":{"input_tokens":36,"output_tokens":6,"cache_read_input_tokens":600}}\n'
}
answer() {
  case "$token" in
    access-expired-state)
      printf '{"type":"assistant","error":"authentication_failed","is_api_error_message":true,"message":{"model":"<synthetic>","content":[{"type":"text","text":"Failed to authenticate"}]}}\n'
      printf '{"type":"result","subtype":"success","is_error":true,"api_error_status":null,"result":"Failed to authenticate","usage":{"input_tokens":0,"output_tokens":0}}\n'
      return
      ;;
  esac
  # The message may come before the context the request adds after it.
  if printf '%s' "$1" | grep -q 'three calls' || grep -q 'three calls' "$session"; then
    calls
    return
  fi
  printf '{"type":"assistant","message":{"content":[{"type":"text","text":"cli=%s token=%s environment=%s descriptor=%s cwd=%s config=%s mode=%s own=%s"}]}}\n' \
    "$0" "$token" "$(env | grep -c "$token")" "$( [ -e /dev/fd/3 ] && echo open || echo none)" "$(pwd)" "$CLAUDE_CONFIG_DIR" "$(ls -ld "$CLAUDE_CONFIG_DIR" | cut -c1-10)" "$own"
  printf '{"type":"result","usage":{"input_tokens":3,"output_tokens":2}}\n'
}
# A session that ends with input is answered at once; one that ends with an
# answer waits for its input, as the CLI does.
last=$(grep -E '"type":"(user|assistant)"' "$session" 2>/dev/null | tail -n 1)
case "$last" in
  *'"type":"user"'*) answer "$last" ;;
esac
while IFS= read -r line; do
  case "$line" in
    '{"type":"control_request"'*'"subtype":"initialize"'*)
      id=$(printf '%s\n' "$line" | sed 's/^{"type":"control_request","request_id":"\([^"]*\)".*/\1/')
      printf '{"type":"control_response","response":{"subtype":"success","request_id":"%s"}}\n' "$id"
      ;;
    '{"type":"control_request"'*'"subtype":"get_usage"'*)
      id=$(printf '%s\n' "$line" | sed 's/^{"type":"control_request","request_id":"\([^"]*\)".*/\1/')
      printf '{"type":"control_response","response":{"subtype":"success","request_id":"%s","response":{"subscription_type":"max","rate_limits":{"five_hour":{"utilization":12,"resets_at":"2030-01-01T00:00:00Z"}}}}}\n' "$id"
      ;;
    # Printed back as it is taken, as `--replay-user-messages` has the CLI
    # do.
    '{"type":"user"'*)
      printf '%s,"isReplay":true}\n' "${line%\}}"
      answer "$line"
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

/// The scripted CLI as `version` of it: each version's bytes differ, as
/// two releases' executables do.
fn scripted(version: &str) -> String {
    format!("{SCRIPTED_CLI}# {version}\n")
}

/// The SHA-256 of `bytes`, in lowercase hex.
fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(bytes))
}

/// Installs `version` of the scripted CLI in `cache`, a runner's artifact
/// cache, as the runner installs a download of it, and answers the
/// executable's path. The cache then removes the line's other versions.
async fn install_by_hand(cache: &std::path::Path, version: &str) -> String {
    let source = tempfile::NamedTempFile::new().unwrap();
    let bytes = scripted(version);
    std::fs::write(source.path(), &bytes).unwrap();
    let cache = std::sync::Arc::new(
        ArtifactCache::new(cache.to_owned(), None, None)
            .await
            .unwrap(),
    );
    let wanted = Wanted {
        package: demi_command_package_claude_code_protocol::PACKAGE,
        name: "Claude Code",
        version,
        artifact: PackageArtifact {
            sha256: sha256(bytes.as_bytes()),
            size: bytes.len() as u64,
        },
        form: &ArtifactForm::File,
    };
    let resolver = std::sync::Arc::new(Local(source.path().to_owned()));
    let path = cache
        .install(&wanted, resolver, SILENT, &CancellationToken::new())
        .await
        .unwrap();
    path.to_string_lossy().into_owned()
}

/// Locates an artifact at a file of this machine.
struct Local(std::path::PathBuf);

impl ArtifactResolver for Local {
    fn resolve<'a>(
        &'a self,
        _: &'a PackageArtifact,
        _: &'a CancellationToken,
    ) -> futures_util::future::BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(std::future::ready(Ok(ArtifactSource::Local(
            self.0.clone(),
        ))))
    }
}

async fn cli(backend: &TestBackend, session: &Session, provider: &str) -> ProviderCli {
    let read = backend
        .get(&format!("/api/providers/{provider}/cli"), Some(session))
        .await;
    assert_eq!(
        read.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&read.body)
    );
    read.json()
}

/// The entry's CLI once no install is under way, asking every 50 ms for at
/// most 30 s.
pub(crate) async fn settled(
    backend: &TestBackend,
    session: &Session,
    provider: &str,
) -> ProviderCli {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        let read = cli(backend, session, provider).await;
        if read.install != Some(CliInstall::Installing {}) {
            return read;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "the install never ended"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A models.dev document whose `anthropic` vendor lists the Claude models
/// the scenarios choose, each leveling its thinking from `low` to `high`.
pub(crate) fn claude_models() -> MockResponse {
    let model = |name: &str| {
        json!({
            "name": name, "reasoning": true, "tool_call": true, "attachment": true,
            "reasoning_options": [{ "type": "effort", "values": ["low", "medium", "high"] }],
            "limit": { "context": 1_000_000, "output": 64_000 }
        })
    };
    let document = json!({
        "anthropic": {
            "id": "anthropic", "name": "Anthropic", "npm": "@ai-sdk/anthropic",
            "models": {
                "claude-opus-4-8": model("Claude Opus 4.8"),
                "claude-sonnet-4-6": model("Claude Sonnet 4.6")
            }
        }
    });
    MockResponse::status(200)
        .header("etag", "\"fixture\"")
        .chunk(document.to_string())
}

/// The conversation's transcript, which holds a turn once the socket has
/// seen it end.
async fn saved(backend: &TestBackend, session: &Session) -> Vec<Block> {
    transcript(backend, session, CONVERSATION).await.blocks
}

/// The Claude Code distribution, which names `NEWEST`, and the catalog the
/// conversations pick their model from.
async fn vendors() -> (MockVendor, MockVendor) {
    let distribution = MockVendor::start().await;
    distribution.respond_at(
        "/releases/latest",
        MockResponse::status(200).chunk(format!("{NEWEST}\n")),
    );
    distribution.respond_at(
        &format!("/releases/{NEWEST}/manifest.json"),
        manifest(NEWEST),
    );
    let catalog = MockVendor::start().await;
    catalog.respond_at("/api.json", claude_models());
    (distribution, catalog)
}

/// The directories the CLI's logins ran in on the machine whose home is
/// `home`, with each one's mode as it ran.
fn login_directories(home: &str) -> Vec<(String, String)> {
    std::fs::read_to_string(format!("{home}/claude-logins.log"))
        .unwrap_or_default()
        .lines()
        .map(|line| {
            let mut fields = line.split(' ').skip(1);
            let directory = fields.next().unwrap().to_owned();
            (directory, fields.next().unwrap_or_default().to_owned())
        })
        .collect()
}

/// A login of the master's into a new Claude Code entry, once it shows its
/// link and waits for a code.
async fn waiting_login(backend: &TestBackend, master: &Session, path: &str, body: Value) -> String {
    let id = start_login(backend, master, path, body).await;
    let shown = awaited(backend, master, &id, |state| {
        !matches!(
            state,
            LoginState::Pending {
                verification_url: None,
                ..
            }
        )
    })
    .await;
    let LoginState::Pending {
        verification_url,
        user_code,
        needs_code,
        ..
    } = shown
    else {
        panic!("the login ended before it showed its link: {shown:?}");
    };
    assert_eq!(
        (verification_url.as_deref(), user_code, needs_code),
        (Some(LINK), None, true)
    );
    id
}

/// Pastes `code` into the master's login `id`, and answers how it ended.
async fn pasted(backend: &TestBackend, master: &Session, id: &str, code: &str) -> LoginState {
    let answer = backend
        .post(
            &format!("{LOGIN}/{id}/code"),
            Some(master),
            json!({ "code": code }),
        )
        .await;
    assert_eq!(
        answer.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&answer.body)
    );
    awaited(backend, master, id, ended).await
}

/// Pastes `code`, which the CLI refuses, into the master's login `id`, and
/// answers the number the login gave it and the words the login then shows
/// while it waits.
async fn refused_code(backend: &TestBackend, master: &Session, id: &str, code: &str) -> (u32, String) {
    let answer = backend
        .post(
            &format!("{LOGIN}/{id}/code"),
            Some(master),
            json!({ "code": code }),
        )
        .await;
    let taken = answer.json::<CodeTaken>().code;
    let refused = awaited(backend, master, id, |state| {
        !matches!(
            state,
            LoginState::Pending {
                code_error: None,
                ..
            }
        )
    })
    .await;
    let LoginState::Pending {
        code_error: Some(words),
        needs_code: true,
        codes_taken,
        ..
    } = refused
    else {
        panic!("the login did not wait for another code: {refused:?}");
    };
    // The refusal is of this code.
    assert_eq!(codes_taken, taken);
    (taken, words)
}

/// What the scripted CLI answers a message with, in a process of the
/// account `account` on the machine whose home is `home`, whose sign-in
/// holds `token`, and whose own directory is `own`.
fn answer(cli: &str, home: &str, token: &str, account: impl std::fmt::Display, own: &str) -> String {
    format!(
        "cli={cli} token={token} environment=0 descriptor=none cwd={home}/run config={home}/accounts/{account} mode=drwx------ own={own}"
    )
}

/// The process's own directory an answer of the scripted CLI names.
fn own_of(answer: &str) -> String {
    let start = answer.find("own=").expect("the answer names its directory") + 4;
    answer[start..].to_owned()
}

// About two seconds: the Claude Code machine boots and installs the
// `demi.claude-code` package, which runs the CLI's logins and three of its
// processes.
#[tokio::test]
async fn a_sign_in_on_the_claude_code_machine_takes_the_pasted_code_and_each_account_infers_in_its_directory()
 {
    let (distribution, catalog) = vendors().await;
    let harness = Harness::new()
        .with_claude_package()
        .with_claude_releases(distribution.url("/releases"))
        .with_models_dev(catalog.url("/api.json"));
    let (backend, master) = harness.start_set_up().await;

    // A sign-in runs Demi's CLI on the Claude Code machine, which has none;
    // the install fails, and the sign-in says why, with the version.
    let id = start_login(
        &backend,
        &master,
        LOGIN,
        json!({ "providerType": "claude-code", "label": "Claude" }),
    )
    .await;
    let LoginState::Failed { message, .. } = awaited(&backend, &master, &id, ended).await else {
        panic!("the sign-in without a CLI did not fail");
    };
    let reason = format!("Claude Code {NEWEST} could not be installed: invalid release record");
    assert!(message.starts_with(&reason), "{message}");
    let providers = backend.get("/api/providers", Some(&master)).await;
    assert!(providers.json::<Providers>().providers.is_empty());
    let machine = harness.manager.claude_code_machine();
    let home = harness.manager.home(&machine);
    let older = install_by_hand(&harness.manager.artifacts(&machine), OLDER).await;
    // The machine is no user's Host: the master's devices do not list it.
    let devices = backend.devices(&master).await;
    assert!(
        devices.iter().all(|device| device.id.to_string() != machine),
        "{devices:?}"
    );

    // The sign-in shows the link the CLI printed and hands the CLI the code
    // the user pastes; the account the CLI wrote is stored, with no token,
    // and the CLI's directory, private to the machine's user, becomes the
    // account's.
    let id = waiting_login(
        &backend,
        &master,
        LOGIN,
        json!({ "providerType": "claude-code", "label": "Claude" }),
    )
    .await;
    let LoginState::Completed {
        provider_id,
        credential_id: first,
    } = pasted(&backend, &master, &id, "first#state").await
    else {
        panic!("the sign-in did not complete");
    };
    let provider = provider_id.as_str().to_owned();
    let logins = login_directories(&home);
    assert_eq!(logins.len(), 1, "{logins:?}");
    assert!(logins[0].0.starts_with(&format!("{home}/accounts/.login-")), "{logins:?}");
    assert_eq!(logins[0].1, "drwx------");
    assert!(!std::path::Path::new(&logins[0].0).exists());
    let first_dir = format!("{home}/accounts/{first}");
    let signed_in = std::fs::read_to_string(format!("{first_dir}/.credentials.json")).unwrap();
    assert!(signed_in.contains("access-first-state"), "{signed_in}");
    // The login ended; it takes no more codes.
    let late = backend
        .post(
            &format!("{LOGIN}/{id}/code"),
            Some(&master),
            json!({ "code": "late#state" }),
        )
        .await;
    assert_eq!(
        late.refusal(),
        (StatusCode::CONFLICT, ErrorCode::LoginNotWaiting)
    );
    let accounts = backend
        .get(
            &format!("/api/providers/{provider}/accounts"),
            Some(&master),
        )
        .await
        .json::<Accounts>();
    assert_eq!(accounts.accounts.len(), 1);
    let account = &accounts.accounts[0];
    assert_eq!(account.label.as_str(), "first@example.test");
    assert!(!account.signed_out);
    assert_eq!(accounts.active.as_ref(), Some(&first));
    let read = cli(&backend, &master, &provider).await;
    assert_eq!(
        read.newest,
        NewestVersion::Read {
            version: NEWEST.into()
        }
    );

    // A request of a conversation whose files are on a paired laptop infers
    // through the Claude Code machine's CLI, in the account's directory,
    // which holds its sign-in, with a directory of its own under it and no
    // token handed to it; nothing of it reaches the laptop.
    create(&backend, &master, CONVERSATION).await;
    let (laptop, _) = on_device(&harness, &backend, &master, CONVERSATION).await;
    choose(
        &backend,
        &master,
        CONVERSATION,
        &provider,
        "claude-opus-4-8",
    )
    .await;
    let mut socket = Socket::connect(&backend, &master, CONVERSATION).await;
    socket.open().await;
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01", "hello")
        .await;
    let first_answer = last_text(&saved(&backend, &master).await);
    let first_own = own_of(&first_answer);
    assert!(
        first_own.starts_with(&format!("{first_dir}/projects/")),
        "{first_answer}"
    );
    assert_eq!(
        first_answer,
        answer(&older, &home, "access-first-state", &first, &first_own)
    );
    assert!(!laptop.runner.home_dir().join("accounts").exists());

    // Another account signs in beside it: a code the CLI refuses keeps the
    // login waiting, with the CLI's words, until the user pastes another.
    // Signing in to the first account again replaces its directory, closing
    // its process, and keeps its ID and the entry's choice.
    let accounts_path = format!("/api/providers/{provider}/accounts");
    let id = waiting_login(&backend, &master, &format!("{accounts_path}/login"), json!({})).await;
    assert_eq!(
        refused_code(&backend, &master, &id, "invalid").await,
        (1, "Invalid code. Please make sure the full code was copied.".to_owned())
    );
    let LoginState::Completed {
        credential_id: second,
        ..
    } = pasted(&backend, &master, &id, "second#state").await
    else {
        panic!("the second sign-in did not complete");
    };
    let id = waiting_login(&backend, &master, &format!("{accounts_path}/login"), json!({})).await;
    let LoginState::Completed {
        credential_id: again,
        ..
    } = pasted(&backend, &master, &id, "first#again").await
    else {
        panic!("signing in to the first account again did not complete");
    };
    assert_eq!(again, first);
    let logins = login_directories(&home);
    assert_eq!(logins.len(), 3, "{logins:?}");
    assert!(
        logins
            .iter()
            .all(|(directory, _)| !std::path::Path::new(directory).exists())
    );
    let signed_in = std::fs::read_to_string(format!("{first_dir}/.credentials.json")).unwrap();
    assert!(signed_in.contains("access-first-again"), "{signed_in}");
    assert!(!std::path::Path::new(&first_own).exists());
    let listed = backend
        .get(&accounts_path, Some(&master))
        .await
        .json::<Accounts>();
    assert_eq!(listed.accounts.len(), 2);
    assert_eq!(listed.active.as_ref(), Some(&first));

    // After the other account is selected, the next request starts a
    // process in that account's directory.
    let selected = backend
        .put(
            &format!("{accounts_path}/active"),
            &master,
            json!({ "credentialId": second }),
        )
        .await;
    assert_eq!(
        selected.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&selected.body)
    );
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a02", "and again")
        .await;
    let second_answer = last_text(&saved(&backend, &master).await);
    let second_own = own_of(&second_answer);
    assert_eq!(
        second_answer,
        answer(&older, &home, "access-second-state", &second, &second_own)
    );

    // Back on the first account, the next request closes that process,
    // whose own directory goes, and a new one finds the new sign-in.
    let selected = backend
        .put(
            &format!("{accounts_path}/active"),
            &master,
            json!({ "credentialId": first }),
        )
        .await;
    assert_eq!(selected.status, StatusCode::OK);
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a03", "once more")
        .await;
    let third_answer = last_text(&saved(&backend, &master).await);
    let third_own = own_of(&third_answer);
    assert_eq!(
        third_answer,
        answer(&older, &home, "access-first-again", &first, &third_own)
    );
    assert!(!std::path::Path::new(&second_own).exists());
    let processes = std::fs::read_to_string(format!("{home}/claude-processes.log")).unwrap();
    let expected = [
        format!("started access-first-state {first_own}"),
        "ended access-first-state".to_owned(),
        format!("started access-second-state {second_own}"),
        "ended access-second-state".to_owned(),
        format!("started access-first-again {third_own}"),
    ];
    assert_eq!(processes.lines().collect::<Vec<_>>(), expected);
    // The distribution was read once: its answer is believed.
    assert_eq!(distribution.requests().len(), 2);
    backend.close().await;
    assert!(!std::path::Path::new(&third_own).exists());
    // The accounts' directories stay: they are the accounts' sign-ins.
    assert!(std::path::Path::new(&first_dir).exists());
}

/// The release pointer holds up only a start that has no CLI to run
/// (`crates/backend-providers/src/llm/claude_releases.rs`): on a machine with none, a sign-in
/// waits for the read and fails with it; on a machine with one, a sign-in
/// and a conversation start it while the read hangs, and the read, in the
/// background, keeps its answer for later starts, which the settings read
/// as it ends rather than reading again. The session describes the machine
/// by the kernel and the `SHELL` its runner reported in its hello: a
/// `uname` first on the machine's `PATH` that names another kernel never
/// runs (§ The session a process resumes).
// About two seconds: the Claude Code machine boots, and runs a sign-in and
// a process of the scripted CLI.
#[tokio::test]
async fn a_machine_with_a_cli_starts_it_at_once_while_the_release_pointer_is_read_in_the_background()
{
    let distribution = MockVendor::start().await;
    let catalog = MockVendor::start().await;
    catalog.respond_at("/api.json", claude_models());
    let shim = tempfile::tempdir().unwrap();
    let uname_runs = shim.path().join("uname-runs.log");
    let uname = shim.path().join("uname");
    std::fs::write(
        &uname,
        format!(
            "#!/bin/sh\necho \"$@\" >> '{}'\necho Shimmed\n",
            uname_runs.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&uname, std::fs::Permissions::from_mode(0o755)).unwrap();
    let harness = Harness::new()
        .with_claude_package()
        .with_claude_releases(distribution.url("/releases"))
        .with_models_dev(catalog.url("/api.json"));
    harness.manager.script(|script| {
        script.cloud_env = Some(BTreeMap::from([
            (
                "PATH".to_owned(),
                format!("{}:/usr/bin:/bin", shim.path().display()),
            ),
            ("SHELL".to_owned(), "/bin/zsh".to_owned()),
        ]));
    });
    let (backend, master) = harness.start_set_up().await;
    let new_entry = json!({ "providerType": "claude-code", "label": "Claude" });

    // The Cloud has no CLI: the sign-in waits for the pointer and fails
    // with its read.
    let (answer_pointer, held) = watch::channel(false);
    distribution.respond_at("/releases/latest", MockResponse::status(503).held(held));
    let id = start_login(&backend, &master, LOGIN, new_entry.clone()).await;
    // The read follows the Claude Code machine's boot, which a first
    // launch of a newly built runner can make longer than `received`'s
    // five seconds: the request is the event, and the deadline guards
    // only against a hang.
    tokio::time::timeout(PATIENCE, distribution.requested(1))
        .await
        .expect("the sign-in reads the pointer");
    assert!(matches!(
        login_state(&backend, &master, &id).await,
        LoginState::Pending { .. }
    ));
    answer_pointer.send_replace(true);
    let LoginState::Failed { message, .. } = awaited(&backend, &master, &id, ended).await else {
        panic!("the sign-in without a CLI did not fail");
    };
    let reason = "Claude Code could not be installed: the distribution answered 503 Service Unavailable";
    assert!(message.starts_with(reason), "{message}");
    let machine = harness.manager.claude_code_machine();
    let home = harness.manager.home(&machine);
    let older = install_by_hand(&harness.manager.artifacts(&machine), OLDER).await;

    // The Cloud has a CLI: the sign-in starts it while the pointer's read,
    // which nothing kept an answer of, hangs.
    let (answer_pointer, held) = watch::channel(false);
    distribution.respond_at(
        "/releases/latest",
        MockResponse::status(200)
            .chunk(format!("{NEWEST}\n"))
            .held(held),
    );
    distribution.respond_at(
        &format!("/releases/{NEWEST}/manifest.json"),
        manifest(NEWEST),
    );
    let id = waiting_login(&backend, &master, LOGIN, new_entry).await;
    distribution.received(2).await;
    let LoginState::Completed { provider_id, .. } =
        pasted(&backend, &master, &id, "first#state").await
    else {
        panic!("the sign-in did not complete");
    };
    let provider = provider_id.as_str().to_owned();

    // So does a conversation's process, whose session describes the Cloud
    // as its runner reported it.
    create(&backend, &master, CONVERSATION).await;
    choose(&backend, &master, CONVERSATION, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, CONVERSATION).await;
    socket.open().await;
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01", "hello")
        .await;
    let said = last_text(&saved(&backend, &master).await);
    let own = own_of(&said);
    let first = said.split("/accounts/").nth(1).unwrap().split(' ').next().unwrap().to_owned();
    assert_eq!(said, answer(&older, &home, "access-first-state", &first, &own));
    let session = std::fs::read_dir(&own)
        .unwrap()
        .next()
        .expect("the process's session is written")
        .unwrap()
        .path();
    let session = std::fs::read_to_string(session).unwrap();
    let environment: Value = serde_json::from_str(session.lines().next().unwrap()).unwrap();
    let uname_of = |flag: &str| {
        let printed = std::process::Command::new("/usr/bin/uname")
            .arg(flag)
            .output()
            .unwrap()
            .stdout;
        String::from_utf8(printed).unwrap().trim().to_owned()
    };
    let (kernel, release) = (uname_of("-s"), uname_of("-r"));
    let run_dir = std::fs::canonicalize(format!("{home}/run")).unwrap();
    assert_eq!(
        environment["attachment"]["snapshot"],
        json!({
            "workingDirectory": run_dir.to_string_lossy(),
            "isWorktree": false,
            "isGitRepo": false,
            "additionalWorkingDirectories": [],
            "platform": kernel.to_lowercase(),
            "shell": "zsh",
            "osVersion": format!("{kernel} {release}"),
        })
    );
    assert!(
        !uname_runs.exists(),
        "a start ran uname: {}",
        std::fs::read_to_string(&uname_runs).unwrap_or_default()
    );

    // The read in the background keeps the pointer's answer: the settings
    // take it once it ends, and nothing reads the pointer again.
    answer_pointer.send_replace(true);
    let read = cli(&backend, &master, &provider).await;
    assert_eq!(
        read.newest,
        NewestVersion::Read {
            version: NEWEST.into()
        }
    );
    assert_eq!(distribution.requests().len(), 3);
    backend.close().await;
}

/// A sign-in that makes its account the entry's active one is a change of
/// the active account, like a selection in Settings
/// (`crates/provider-claude-code/src/prepared.rs`): the open conversation on
/// the entry
/// prepares its next process with the new account's token, and its next
/// message takes that process. Removing the active account, the last, closed
/// the process that held the previous token first.
// About two seconds: the Cloud boots, and runs two sign-ins and two
// processes of the scripted CLI.
#[tokio::test]
async fn a_sign_in_that_activates_its_account_prepares_the_open_conversation_with_its_token() {
    let (distribution, catalog) = vendors().await;
    let harness = Harness::new()
        .with_claude_package()
        .with_claude_releases(distribution.url("/releases"))
        .with_models_dev(catalog.url("/api.json"));
    let (backend, master) = harness.start_set_up().await;
    // A first sign-in boots the Cloud, which has no CLI yet.
    let id = start_login(
        &backend,
        &master,
        LOGIN,
        json!({ "providerType": "claude-code", "label": "Claude" }),
    )
    .await;
    awaited(&backend, &master, &id, ended).await;
    let machine = harness.manager.claude_code_machine();
    let home = harness.manager.home(&machine);
    let older = install_by_hand(&harness.manager.artifacts(&machine), OLDER).await;
    let id = waiting_login(
        &backend,
        &master,
        LOGIN,
        json!({ "providerType": "claude-code", "label": "Claude" }),
    )
    .await;
    let LoginState::Completed {
        provider_id,
        credential_id: first,
    } = pasted(&backend, &master, &id, "first#state").await
    else {
        panic!("the sign-in did not complete");
    };
    let provider = provider_id.as_str().to_owned();
    create(&backend, &master, CONVERSATION).await;
    choose(&backend, &master, CONVERSATION, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, CONVERSATION).await;
    socket.open().await;
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01", "hello")
        .await;
    let first_own = own_of(&last_text(&saved(&backend, &master).await));

    // The entry's last account goes, and with it the process of its token.
    let accounts_path = format!("/api/providers/{provider}/accounts");
    let removed = backend
        .delete(&format!("{accounts_path}/{first}"), &master)
        .await;
    assert_eq!(removed.status, StatusCode::OK);

    // Signing in again makes the new account the active one: the open
    // conversation prepares its next process with its token.
    let id = waiting_login(&backend, &master, &format!("{accounts_path}/login"), json!({})).await;
    let LoginState::Completed {
        credential_id: second,
        ..
    } = pasted(&backend, &master, &id, "second#state").await
    else {
        panic!("the second sign-in did not complete");
    };
    tokio::time::timeout(Duration::from_secs(10), backend.until_prepared(&master, 1))
        .await
        .expect("the sign-in's account prepared the open conversation's process");
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a02", "and again")
        .await;
    let second_answer = last_text(&saved(&backend, &master).await);
    let second_own = own_of(&second_answer);
    assert_eq!(
        second_answer,
        answer(&older, &home, "access-second-state", &second, &second_own)
    );
    // The first process went with its account, and the second, started
    // before the message, answered it.
    let processes = std::fs::read_to_string(format!("{home}/claude-processes.log")).unwrap();
    let expected = [
        format!("started access-first-state {first_own}"),
        "ended access-first-state".to_owned(),
        format!("started access-second-state {second_own}"),
    ];
    assert_eq!(processes.lines().collect::<Vec<_>>(), expected);
    backend.close().await;
}

/// A Claude Code run of three calls to the vendor is three usage ledger
/// rows, one per call, each with its message's id, its own counts and the
/// account, never the `result` line's sum
/// (`crates/backend-database/src/usage.rs`); the quota the vendor reported
/// after the last call's end is that
/// call's alone; the server log's lines of each call name its request, its agent
/// and its account, and so does the run's timing line.
// About two seconds: the Claude Code machine boots, and runs a sign-in and
// a process of the scripted CLI.
#[tokio::test]
async fn a_claude_code_run_of_three_calls_is_three_ledger_rows_with_the_account() {
    let (distribution, catalog) = vendors().await;
    let harness = Harness::new()
        .with_claude_package()
        .with_claude_releases(distribution.url("/releases"))
        .with_models_dev(catalog.url("/api.json"));
    let log = ServerLogCapture::open(&harness);
    let (backend, master) = harness.start_set_up().await;
    // A first sign-in boots the Claude Code machine, which has no CLI yet.
    let id = start_login(
        &backend,
        &master,
        LOGIN,
        json!({ "providerType": "claude-code", "label": "Claude" }),
    )
    .await;
    awaited(&backend, &master, &id, ended).await;
    let machine = harness.manager.claude_code_machine();
    install_by_hand(&harness.manager.artifacts(&machine), OLDER).await;
    let id = waiting_login(
        &backend,
        &master,
        LOGIN,
        json!({ "providerType": "claude-code", "label": "Claude" }),
    )
    .await;
    let LoginState::Completed {
        provider_id,
        credential_id: account,
    } = pasted(&backend, &master, &id, "first#state").await
    else {
        panic!("the sign-in did not complete");
    };
    create(&backend, &master, CONVERSATION).await;
    choose(&backend, &master, CONVERSATION, provider_id.as_str(), "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, CONVERSATION).await;
    socket.open().await;
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01", "answer in three calls")
        .await;

    let rows = crate::usage::rows(&harness);
    let calls: Vec<_> = rows
        .iter()
        .map(|row| {
            (
                row.vendor_id.as_deref(),
                row.tokens,
                row.cache_write_1h_tokens,
                row.outcome.as_deref(),
            )
        })
        .collect();
    assert_eq!(
        calls,
        [
            (Some("msg_call_1"), [Some(11), Some(1), Some(100), Some(10)], Some(1), Some("answered")),
            (Some("msg_call_2"), [Some(12), Some(2), Some(200), Some(20)], Some(2), Some("answered")),
            (Some("msg_call_3"), [Some(13), Some(3), Some(300), Some(30)], Some(3), Some("answered")),
        ]
    );
    let quota: Vec<Option<Value>> = rows
        .iter()
        .map(|row| row.quota.as_deref().map(|quota| serde_json::from_str(quota).unwrap()))
        .collect();
    assert_eq!(
        quota,
        [
            None,
            None,
            Some(json!([
                { "kind": "limit", "periodSeconds": 18_000, "scope": null, "usedPercent": 31.0,
                  "resetsAt": "2026-10-13T09:10:00.000Z" },
                { "kind": "limit", "periodSeconds": 604_800, "scope": null, "usedPercent": 5.0,
                  "resetsAt": "2026-10-19T22:00:00.000Z" },
            ])),
        ]
    );
    let first = &rows[0];
    assert!(
        rows.iter().all(|row| row.account_id.as_deref() == Some(account.as_str())
            && row.agent == Some(0)
            && row.purpose.as_deref() == Some("turn")
            && row.turn_id == first.turn_id
            && row.request_id == first.request_id
            && row.provider_id == provider_id.as_str()),
        "{rows:?}"
    );
    drop(socket);
    backend.close().await;
    log.close().await;
    let lines = crate::usage::logged(&harness);
    let call_lines: Vec<_> = lines
        .iter()
        .filter(|(message, _)| message == "a vendor call ended")
        .collect();
    assert_eq!(call_lines.len(), 3, "{lines:?}");
    let timing = lines
        .iter()
        .filter(|(message, _)| message == "Claude Code run ended")
        .collect::<Vec<_>>();
    assert!(!timing.is_empty(), "{lines:?}");
    for (_, fields) in call_lines.iter().chain(&timing) {
        assert_eq!(fields["request"], first.request_id.as_deref().unwrap(), "{fields}");
        assert_eq!(fields["agent"], 0, "{fields}");
        assert_eq!(fields["account"], account.as_str(), "{fields}");
    }
}

// An expired sign-in stops the CLI as a cancelled one does
// (`LoginFlows::run`); the expiry itself, counted from the start, is proven
// on a paused clock by the test in `vault::logins`. Proving it here would
// take a lifetime short enough to wait for, a window that each sign-in's CLI
// start and pasted code had to fall into, and a loaded machine misses it.
#[tokio::test]
async fn a_sign_in_that_fails_or_is_cancelled_stores_nothing_and_leaves_no_directory() {
    let (distribution, catalog) = vendors().await;
    let harness = Harness::new()
        .with_claude_package()
        .with_claude_releases(distribution.url("/releases"))
        .with_models_dev(catalog.url("/api.json"));
    let (backend, master) = harness.start_set_up().await;
    let new_entry = json!({ "providerType": "claude-code" });
    // The first sign-in wakes the Claude Code machine, which has no CLI to
    // run.
    let id = start_login(&backend, &master, LOGIN, new_entry.clone()).await;
    assert!(matches!(
        awaited(&backend, &master, &id, ended).await,
        LoginState::Failed { .. }
    ));
    let machine = harness.manager.claude_code_machine();
    let home = harness.manager.home(&machine);
    install_by_hand(&harness.manager.artifacts(&machine), OLDER).await;

    // The CLI's own words of failure are what the user sees.
    let id = waiting_login(&backend, &master, LOGIN, new_entry.clone()).await;
    assert!(matches!(
        pasted(&backend, &master, &id, "refused#state").await,
        LoginState::Failed { message, .. } if message == "Login failed: the code was refused"
    ));

    // A cancelled sign-in has ended, with its directory gone, once the
    // cancel answers.
    let id = waiting_login(&backend, &master, LOGIN, new_entry).await;
    let cancelled = backend.delete(&format!("{LOGIN}/{id}"), &master).await;
    assert_eq!(cancelled.status, StatusCode::NO_CONTENT);
    assert!(matches!(
        login_state(&backend, &master, &id).await,
        LoginState::Cancelled
    ));
    let logins = login_directories(&home);
    assert_eq!(logins.len(), 2, "{logins:?}");
    assert!(
        logins
            .iter()
            .all(|(directory, _)| !std::path::Path::new(directory).exists())
    );
    let providers = backend.get("/api/providers", Some(&master)).await;
    assert!(providers.json::<Providers>().providers.is_empty());
    backend.close().await;
}

/// The course the server log holds of the login the request `request`
/// started, once it holds the login's end: that it started, the stages it
/// reached in order, each with the time the one before took, and how it
/// ended; no line holds the link the CLI printed.
async fn logged(backend: &TestBackend, admin: &Session, request: &str) -> (Vec<String>, String) {
    let ended = |line: &ServerLogLine| line.message.starts_with("the login ") && line.message != "the login reached a stage";
    eventually("the login's end is in the log", || async {
        lines_of(backend, admin, request).await.lines.iter().any(ended)
    })
    .await;
    let lines = lines_of(backend, admin, request).await.lines;
    let text = serde_json::to_string(&lines).unwrap();
    assert!(!text.contains("oauth/authorize"), "{text}");
    assert!(lines.iter().any(|line| line.message == "a login started"), "{text}");
    let stages = lines
        .iter()
        .filter(|line| line.message == "the login reached a stage")
        .map(|line| {
            assert!(line.fields["took_ms"].is_u64(), "{line:?}");
            line.fields["stage"].as_str().unwrap().to_owned()
        })
        .collect();
    let end = lines.iter().find(|line| ended(line)).unwrap().message.clone();
    (stages, end)
}

/// Whether `stages` go in the order a sign-in reaches them, as a page
/// sees them: the channel may pass over a stage the login left before the
/// channel sent it, never go back to one.
fn in_order(stages: &[LoginStage]) -> bool {
    let rank = |stage: &LoginStage| match stage {
        LoginStage::Preparing => 0,
        LoginStage::WaitingForMachine => 1,
        LoginStage::Installing => 2,
        LoginStage::Starting => 3,
        LoginStage::WaitingForUser => 4,
    };
    stages.windows(2).all(|pair| rank(&pair[0]) < rank(&pair[1]))
}

/// Starts the master's sign-in into a new Claude Code entry, and answers
/// its ID and the ID of the request that started it.
async fn started_sign_in(backend: &TestBackend, master: &Session) -> (String, String) {
    let answer = backend
        .post(LOGIN, Some(master), json!({ "providerType": "claude-code" }))
        .await;
    assert_eq!(answer.status, StatusCode::ACCEPTED);
    let request = request_id(&answer);
    (answer.json::<LoginStarted>().login.id.into_string(), request)
}

// Several seconds: the frozen machine's runner timeout passes in real time,
// then the machine boots and the CLI's login starts.
#[tokio::test]
async fn a_sign_in_says_what_it_waits_for_and_fails_at_the_stage_that_did_not_finish() {
    let (distribution, catalog) = vendors().await;
    let mut harness = Harness::new()
        .with_claude_package()
        .with_claude_releases(distribution.url("/releases"))
        .with_models_dev(catalog.url("/api.json"));
    // The timeout of a machine whose runner never connects, short enough to
    // wait for.
    harness.cloud.runner_connection = Duration::from_secs(2);
    let log = ServerLogCapture::open(&harness);
    let (backend, master) = harness.start_set_up().await;
    let mut page = backend.sync(&master).await;
    page.snapshot().await;

    // A Claude Code machine whose runner never connects: the sign-in waits
    // for the machine, and fails there, saying so, with the request that
    // started it.
    harness.manager.script(|script| script.silent_wake = true);
    let (id, request) = started_sign_in(&backend, &master).await;
    let (stages, end) = followed(&mut page, &id).await;
    assert!(in_order(&stages), "{stages:?}");
    assert_eq!(stages.last(), Some(&LoginStage::WaitingForMachine), "{stages:?}");
    assert_eq!(
        end,
        LoginState::Failed {
            message: "Claude Code’s machine didn’t start. Its runner didn’t connect in 2 seconds."
                .into(),
            request_id: request.clone().try_into().unwrap(),
        }
    );
    assert_eq!(
        logged(&backend, &master, &request).await,
        (vec!["waiting_for_machine".to_owned()], "the login failed".to_owned())
    );

    // A machine without the CLI installs it, and fails there.
    harness.manager.script(|script| script.silent_wake = false);
    let (id, request) = started_sign_in(&backend, &master).await;
    let (stages, end) = followed(&mut page, &id).await;
    assert!(in_order(&stages), "{stages:?}");
    assert_eq!(stages.last(), Some(&LoginStage::Installing), "{stages:?}");
    let LoginState::Failed { message, request_id } = end else {
        panic!("the install did not fail: {end:?}");
    };
    assert!(message.starts_with(&format!("Claude Code {NEWEST} could not be installed")), "{message}");
    assert_eq!(request_id.as_str(), request);
    assert_eq!(
        logged(&backend, &master, &request).await,
        (
            vec!["waiting_for_machine".to_owned(), "installing".to_owned()],
            "the login failed".to_owned()
        )
    );

    // A machine with the CLI starts it, and waits for the user once the CLI
    // printed its link.
    let machine = harness.manager.claude_code_machine();
    install_by_hand(&harness.manager.artifacts(&machine), OLDER).await;
    let (id, request) = started_sign_in(&backend, &master).await;
    let mut stages = Vec::new();
    loop {
        let SyncEvent::Logins { logins } = page.next().await else {
            continue;
        };
        let Some(LoginState::Pending { stage, verification_url, .. }) =
            logins.into_iter().find(|login| login.id.as_str() == id).map(|login| login.login)
        else {
            continue;
        };
        if stages.last() != Some(&stage) {
            stages.push(stage);
        }
        if verification_url.is_some() {
            assert_eq!(stage, LoginStage::WaitingForUser);
            break;
        }
    }
    assert!(in_order(&stages), "{stages:?}");
    assert!(!stages.contains(&LoginStage::Installing), "{stages:?}");
    let cancelled = backend.delete(&format!("{LOGIN}/{id}"), &master).await;
    assert_eq!(cancelled.status, StatusCode::NO_CONTENT);
    assert_eq!(
        logged(&backend, &master, &request).await,
        (
            vec!["waiting_for_machine".to_owned(), "starting".to_owned(), "waiting_for_user".to_owned()],
            "the login was cancelled".to_owned()
        )
    );
    backend.close().await;
    log.close().await;
}

/// The kept versions' scenarios: a Cloud whose runner shares the artifact
/// cache `cache`, and the backend with its master, a Claude Code entry
/// whose account is stored as a sign-in stores it, and its server log.
struct Kept {
    harness: Harness,
    backend: TestBackend,
    master: Session,
    provider: String,
    log: ServerLogCapture,
}

impl Kept {
    async fn start(distribution: &MockVendor, catalog: &MockVendor, cache: &std::path::Path) -> Self {
        let harness = Harness::new()
            .with_claude_package()
            .with_claude_releases(distribution.url("/releases"))
            .with_models_dev(catalog.url("/api.json"));
        let shared = cache.to_string_lossy().into_owned();
        harness
            .manager
            .script(|script| script.artifacts = Some(shared));
        let log = ServerLogCapture::open(&harness);
        let (backend, master) = harness.start_set_up().await;
        let secret = json!({
            "accessToken": "access-kept",
            "refreshToken": "refresh-kept",
            "expiresAt": "2100-01-01T00:00:00.000Z",
            "scopes": ["user:inference", "user:profile"],
            "subscriptionType": "max",
            "accountId": "kept",
            "email": "kept@example.test",
        });
        let provider = backend
            .seed_subscription(&master, "claude-code", "Claude", "account:kept", secret.to_string())
            .await;
        Self {
            harness,
            backend,
            master,
            provider,
            log,
        }
    }

    /// Says hello in the new conversation `n` on the entry, and answers
    /// what its CLI process said, or why the turn failed.
    async fn said(&self, n: u8) -> String {
        let id = format!("6a1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a{n:02}");
        create(&self.backend, &self.master, &id).await;
        choose(&self.backend, &self.master, &id, &self.provider, "claude-opus-4-8").await;
        let mut socket = Socket::connect(&self.backend, &self.master, &id).await;
        socket.open().await;
        socket.chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01", "hello").await;
        let blocks = transcript(&self.backend, &self.master, &id).await.blocks;
        match blocks.last() {
            Some(Block::Error(failed)) => format!("failed: {}", failed.message),
            _ => last_text(&blocks),
        }
    }

    /// Whether the server log holds a line whose message starts with
    /// `words`.
    async fn holds(&self, words: &str) -> bool {
        let read = self
            .backend
            .get("/api/server/log?limit=1000", Some(&self.master))
            .await;
        assert_eq!(read.status, StatusCode::OK);
        read.json::<ServerLog>()
            .lines
            .iter()
            .any(|line| line.message.starts_with(words))
    }

    /// Once the server log holds a line whose message starts with `words`.
    async fn logged(&self, words: &str) {
        eventually(words, || self.holds(words)).await;
    }

    async fn close(self) {
        self.backend.close().await;
        self.log.close().await;
    }
}

/// The Cloud's `demi.claude-code` service, which a test freezes so that
/// no operation of the package answers, or kills as a crash would. One
/// left frozen goes on when the test ends.
pub(crate) struct PackageService {
    pid: rustix::process::Pid,
    frozen: bool,
}

impl PackageService {
    /// The service the Cloud's runner started last.
    pub(crate) fn of(harness: &Harness) -> Self {
        let log = harness.manager.log(&harness.manager.claude_code_machine());
        let started = format!(
            "service {} started (pid ",
            demi_command_package_claude_code_protocol::PACKAGE
        );
        let pid = log
            .lines()
            .rev()
            .find_map(|line| {
                let after = &line[line.find(&started)? + started.len()..];
                after.split(')').next()?.parse().ok()
            })
            .and_then(rustix::process::Pid::from_raw)
            .unwrap_or_else(|| panic!("the Cloud's runner started no package service: {log}"));
        Self { pid, frozen: false }
    }

    fn signal(&self, signal: rustix::process::Signal) {
        rustix::process::kill_process(self.pid, signal).unwrap();
    }

    pub(crate) fn freeze(&mut self) {
        self.signal(rustix::process::Signal::STOP);
        self.frozen = true;
    }

    fn crash(mut self) {
        self.signal(rustix::process::Signal::KILL);
        self.frozen = false;
    }
}

impl Drop for PackageService {
    fn drop(&mut self) {
        if self.frozen {
            self.signal(rustix::process::Signal::CONT);
        }
    }
}

/// The versions a Cloud has are asked once per runner connection, and
/// asked again at the next start once the package's service ended unsaid
/// or the connection ended (`crates/backend-providers/src/llm/claude_releases.rs`): while the
/// package cannot answer, another conversation starts the version the
/// first one's start was told of; a version installed by hand meanwhile,
/// which removed the one before, is what starts after each.
// A few seconds: the Cloud boots twice and starts four CLI processes and
// those prepared ahead.
#[tokio::test]
async fn a_clouds_versions_are_asked_once_per_connection_and_again_after_a_lost_package_or_a_new_connection() {
    let cache = tempfile::tempdir().unwrap();
    // No release is ever believed, so nothing starts an install.
    let distribution = MockVendor::start().await;
    distribution.respond_at("/releases/latest", MockResponse::status(503));
    let catalog = MockVendor::start().await;
    catalog.respond_at("/api.json", claude_models());
    let older = install_by_hand(cache.path(), OLDER).await;
    let kept = Kept::start(&distribution, &catalog, cache.path()).await;
    let said = kept.said(1).await;
    assert!(said.starts_with(&format!("cli={older} ")), "{said}");
    let mut service = PackageService::of(&kept.harness);
    service.freeze();
    let said = kept.said(2).await;
    assert!(said.starts_with(&format!("cli={older} ")), "{said}");

    // The package's service crashes: the runner says so, and the next start
    // asks.
    let newest = install_by_hand(cache.path(), NEWEST).await;
    service.crash();
    kept.logged("the Claude Code versions kept for the device were dropped: its package was lost")
        .await;
    let said = kept.said(3).await;
    assert!(said.starts_with(&format!("cli={newest} ")), "{said}");

    // The runner crashes: the next start wakes the Cloud and asks over the
    // new connection.
    let third = install_by_hand(cache.path(), "2.1.4").await;
    kept.harness.manager.kill(&kept.harness.manager.claude_code_machine()).await;
    let said = kept.said(4).await;
    assert!(said.starts_with(&format!("cli={third} ")), "{said}");
    kept.close().await;
}

/// A first ask of the Claude Code machine's versions that every start
/// waiting for it gave up on is dropped with them
/// (`crates/backend-providers/src/llm/claude_releases.rs`). An update to the
/// newest release fails, as the distribution's
/// address is not HTTPS, which drops what is kept while the machine's
/// package service runs on; a line record that is a FIFO holds the
/// service's listing of the versions, so the one ask the starts of two
/// conversations share hangs, and both turns are stopped. The hung ask then
/// answers with a version that is gone; the next start asks again rather
/// than take that answer, and starts the CLI without finding an executable
/// gone.
// A second or two: the machine boots and starts two CLI processes and those
// prepared ahead.
#[tokio::test]
async fn concurrent_first_starts_share_one_ask_and_one_they_all_gave_up_on_is_asked_again() {
    use rustix::fs::{Mode, OFlags};

    let cache = tempfile::tempdir().unwrap();
    let (distribution, catalog) = vendors().await;
    let older = install_by_hand(cache.path(), OLDER).await;
    let kept = Kept::start(&distribution, &catalog, cache.path()).await;
    let said = kept.said(1).await;
    assert!(said.starts_with(&format!("cli={older} ")), "{said}");
    let record = cache.path().join(format!("{}.line.json", "ef".repeat(32)));
    let made = std::process::Command::new("mkfifo").arg(&record).status().unwrap();
    assert!(made.success());
    // The settings read the newest release, which **Install** then fails to
    // install beside the one the Cloud has.
    let read = cli(&kept.backend, &kept.master, &kept.provider).await;
    assert_eq!(read.newest, NewestVersion::Read { version: NEWEST.into() });
    let started = kept
        .backend
        .post(&format!("/api/providers/{}/cli/install", kept.provider), Some(&kept.master), json!({}))
        .await;
    assert_eq!(started.status, StatusCode::ACCEPTED);
    kept.logged("the Claude Code machine's CLI was not updated").await;

    // Two conversations send a message. The listing opens the record and
    // waits for bytes: a writer that opens it without blocking finds the
    // ask there, and holds it hung.
    let mut sockets = Vec::new();
    for n in [3u8, 4] {
        let id = format!("6a1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a{n:02}");
        create(&kept.backend, &kept.master, &id).await;
        choose(&kept.backend, &kept.master, &id, &kept.provider, "claude-opus-4-8").await;
        let mut socket = Socket::connect(&kept.backend, &kept.master, &id).await;
        socket.open().await;
        socket.send(&send("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01", "hello")).await;
        sockets.push(socket);
    }
    let asked = async {
        loop {
            match rustix::fs::open(&record, OFlags::WRONLY | OFlags::NONBLOCK | OFlags::CLOEXEC, Mode::empty()) {
                Ok(writer) => break writer,
                Err(rustix::io::Errno::NXIO) => tokio::time::sleep(Duration::from_millis(20)).await,
                Err(error) => panic!("the record could not be opened: {error}"),
            }
        }
    };
    let writer = tokio::time::timeout(PATIENCE, asked)
        .await
        .expect("the ask reaches the record");
    for socket in &mut sockets {
        socket.stop().await;
    }
    kept.logged("the ask of the device's Claude Code versions was given up by every start that waited for it")
        .await;
    // The two starts shared one ask: the machine ran the package's status
    // twice in all, for the first start and for them.
    let statuses = kept
        .harness
        .manager
        .log(&kept.harness.manager.claude_code_machine())
        .matches("stream:claude-code.status opened")
        .count();
    assert_eq!(statuses, 2);

    // The FIFO leaves the cache, and the hung listing reads from it a record
    // of a newer version whose executable is gone: the hung ask's answer
    // names it, and a start that took that answer would find it gone.
    let held = tempfile::tempdir().unwrap();
    std::fs::rename(&record, held.path().join("fifo")).unwrap();
    let gone = json!({
        "package": demi_command_package_claude_code_protocol::PACKAGE,
        "name": "Claude Code",
        "version": "9.9.9",
        "path": held.path().join("claude"),
        "installedAt": u64::MAX,
    });
    rustix::io::write(&writer, gone.to_string().as_bytes()).unwrap();
    drop(writer);
    let said = kept.said(2).await;
    assert!(said.starts_with(&format!("cli={older} ")), "{said}");
    assert!(
        !kept.holds("the Claude Code versions kept for the device were dropped: its executable was gone").await,
        "a start took the hung ask's answer"
    );
    kept.close().await;
}

/// A start that finds the executable it was given gone drops what is
/// kept (`crates/backend-providers/src/llm/claude_releases.rs`): after a runner sharing the
/// cache installed another version and removed the one a start was told
/// of, the next start asks again and starts the new one.
// A second or two: the Cloud boots and starts a few CLI processes and
// those prepared ahead.
#[tokio::test]
async fn a_start_that_finds_its_executable_gone_makes_the_next_ask_again() {
    let cache = tempfile::tempdir().unwrap();
    let distribution = MockVendor::start().await;
    distribution.respond_at("/releases/latest", MockResponse::status(503));
    let catalog = MockVendor::start().await;
    catalog.respond_at("/api.json", claude_models());
    let older = install_by_hand(cache.path(), OLDER).await;
    let kept = Kept::start(&distribution, &catalog, cache.path()).await;
    let said = kept.said(1).await;
    assert!(said.starts_with(&format!("cli={older} ")), "{said}");

    let newest = install_by_hand(cache.path(), NEWEST).await;
    assert!(!std::path::Path::new(&older).exists());
    // This start fails, unless a process prepared ahead for the first
    // conversation found the executable gone before it.
    let said = kept.said(2).await;
    assert!(said.starts_with("failed: ") || said.starts_with(&format!("cli={newest} ")), "{said}");
    kept.logged("the Claude Code versions kept for the device were dropped: its executable was gone")
        .await;
    let said = kept.said(3).await;
    assert!(said.starts_with(&format!("cli={newest} ")), "{said}");
    kept.close().await;
}

/// A failed install drops what is kept (`crates/backend-providers/src/llm/claude_releases.rs`):
/// **Install** on a Cloud with none fails, the CLI is then installed by
/// hand, and the next start asks and starts it.
// A few seconds: the Cloud boots and starts a CLI process and the one
// prepared ahead.
#[tokio::test]
async fn a_failed_install_makes_the_next_start_ask_the_cloud_again() {
    let cache = tempfile::tempdir().unwrap();
    let (distribution, catalog) = vendors().await;
    let kept = Kept::start(&distribution, &catalog, cache.path()).await;
    let started = kept
        .backend
        .post(&format!("/api/providers/{}/cli/install", kept.provider), Some(&kept.master), json!({}))
        .await;
    assert_eq!(started.status, StatusCode::ACCEPTED);
    let failed = settled(&kept.backend, &kept.master, &kept.provider).await.install;
    assert!(matches!(failed, Some(CliInstall::Failed { .. })), "{failed:?}");
    let older = install_by_hand(cache.path(), OLDER).await;
    let said = kept.said(1).await;
    assert!(said.starts_with(&format!("cli={older} ")), "{said}");
    kept.close().await;
}

/// An account an earlier release stored with its tokens hands them to its
/// directory at its first use (`crates/provider-claude-code/src/account.rs`,
/// `crates/server/src/main.rs`): the first start writes them as the CLI's
/// own files, private to the machine's user, and the record keeps none, so
/// the next start finds the sign-in in the directory alone.
// A second or two: the machine boots and starts two CLI processes and those
// prepared ahead.
#[tokio::test]
async fn an_earlier_releases_sign_in_goes_to_the_accounts_directory_at_its_first_use() {
    use std::os::unix::fs::MetadataExt as _;

    let cache = tempfile::tempdir().unwrap();
    let (distribution, catalog) = vendors().await;
    let older = install_by_hand(cache.path(), OLDER).await;
    let kept = Kept::start(&distribution, &catalog, cache.path()).await;
    let said = kept.said(1).await;
    assert!(said.starts_with(&format!("cli={older} token=access-kept ")), "{said}");
    let accounts = kept
        .backend
        .get(&format!("/api/providers/{}/accounts", kept.provider), Some(&kept.master))
        .await
        .json::<Accounts>();
    let account = accounts.active.unwrap();
    let home = kept.harness.manager.home(&kept.harness.manager.claude_code_machine());
    let directory = format!("{home}/accounts/{account}");
    let file = |name: &str| {
        let path = format!("{directory}/{name}");
        let mode = std::fs::metadata(&path).unwrap().mode() & 0o777;
        let text = std::fs::read_to_string(&path).unwrap();
        (mode, serde_json::from_str::<Value>(&text).unwrap())
    };
    assert_eq!(
        file(".credentials.json"),
        (
            0o600,
            json!({ "claudeAiOauth": {
                "accessToken": "access-kept",
                "refreshToken": "refresh-kept",
                "expiresAt": 4_102_444_800_000_i64,
                "scopes": ["user:inference", "user:profile"],
                "subscriptionType": "max",
            } })
        )
    );
    assert_eq!(
        file(".claude.json"),
        (
            0o600,
            json!({ "oauthAccount": { "accountUuid": "kept", "emailAddress": "kept@example.test" } })
        )
    );
    let secret: Option<Vec<u8>> = kept
        .harness
        .control_database()
        .query_row("SELECT secret FROM provider_credentials", [], |row| row.get(0))
        .unwrap();
    assert_eq!(secret, None);
    let said = kept.said(2).await;
    assert!(said.starts_with(&format!("cli={older} token=access-kept ")), "{said}");
    kept.close().await;
}

/// A sign-in the vendor refused signs its account out
/// (`crates/provider-claude-code/src/account.rs`): the turn fails with the
/// authentication error,
/// whose words ask the user to sign in again, the accounts list says the
/// account is signed out, and signing in to it again clears that, after
/// which its requests run on the new sign-in.
// About two seconds: the machine boots, and runs two sign-ins and two
// processes of the scripted CLI.
#[tokio::test]
async fn a_sign_in_the_vendor_refused_signs_the_account_out_until_it_signs_in_again() {
    let (distribution, catalog) = vendors().await;
    let harness = Harness::new()
        .with_claude_package()
        .with_claude_releases(distribution.url("/releases"))
        .with_models_dev(catalog.url("/api.json"));
    let (backend, master) = harness.start_set_up().await;
    let new_entry = json!({ "providerType": "claude-code", "label": "Claude" });
    let id = start_login(&backend, &master, LOGIN, new_entry.clone()).await;
    awaited(&backend, &master, &id, ended).await;
    let machine = harness.manager.claude_code_machine();
    let home = harness.manager.home(&machine);
    let older = install_by_hand(&harness.manager.artifacts(&machine), OLDER).await;
    let id = waiting_login(&backend, &master, LOGIN, new_entry).await;
    let LoginState::Completed {
        provider_id,
        credential_id: account,
    } = pasted(&backend, &master, &id, "expired#state").await
    else {
        panic!("the sign-in did not complete");
    };
    let provider = provider_id.as_str().to_owned();
    let accounts_path = format!("/api/providers/{provider}/accounts");
    create(&backend, &master, CONVERSATION).await;
    choose(&backend, &master, CONVERSATION, &provider, "claude-opus-4-8").await;
    let mut socket = Socket::connect(&backend, &master, CONVERSATION).await;
    socket.open().await;
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01", "hello")
        .await;
    let blocks = saved(&backend, &master).await;
    let Some(Block::Error(failed)) = blocks.last() else {
        panic!("the refused sign-in did not fail the turn: {blocks:?}");
    };
    assert_eq!(failed.message, "Claude Code signed the account out. Sign in to it again");
    let listed = backend.get(&accounts_path, Some(&master)).await.json::<Accounts>();
    assert!(listed.accounts[0].signed_out, "{:?}", listed.accounts);

    // Signing in to it again clears the mark, and its next request runs.
    let id = waiting_login(&backend, &master, &format!("{accounts_path}/login"), json!({})).await;
    let LoginState::Completed {
        credential_id: again,
        ..
    } = pasted(&backend, &master, &id, "expired#again").await
    else {
        panic!("signing in again did not complete");
    };
    assert_eq!(again, account);
    let listed = backend.get(&accounts_path, Some(&master)).await.json::<Accounts>();
    assert!(!listed.accounts[0].signed_out, "{:?}", listed.accounts);
    socket
        .chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a02", "again")
        .await;
    let said = last_text(&saved(&backend, &master).await);
    let own = own_of(&said);
    assert_eq!(said, answer(&older, &home, "access-expired-again", &account, &own));
    backend.close().await;
}

/// Whether the server log holds a line whose message starts with `words`.
async fn server_holds(backend: &TestBackend, master: &Session, words: &str) -> bool {
    let read = backend.get("/api/server/log?limit=1000", Some(master)).await;
    assert_eq!(read.status, StatusCode::OK);
    read.json::<ServerLog>()
        .lines
        .iter()
        .any(|line| line.message.starts_with(words))
}

/// One change of an account at a time
/// (`crates/provider-claude-code/src/account.rs`): signing an account in again
/// first stops new processes of the
/// account, closes the one it has and waits for it, then replaces its
/// directory, and a request that came meanwhile starts in the new one; a
/// login stays cancellable while it waits for the account's processes, and
/// leaves the account as it was; removing the account closes its process
/// and deletes its directory.
// A few seconds: the machine boots, runs four sign-ins and three processes
// of the scripted CLI, one of which takes its time to end.
#[tokio::test]
async fn a_change_of_an_account_closes_its_processes_first_and_a_request_meanwhile_waits_for_it() {
    let (distribution, catalog) = vendors().await;
    let harness = Harness::new()
        .with_claude_package()
        .with_claude_releases(distribution.url("/releases"))
        .with_models_dev(catalog.url("/api.json"));
    let log = ServerLogCapture::open(&harness);
    let (backend, master) = harness.start_set_up().await;
    let new_entry = json!({ "providerType": "claude-code", "label": "Claude" });
    let id = start_login(&backend, &master, LOGIN, new_entry.clone()).await;
    awaited(&backend, &master, &id, ended).await;
    let machine = harness.manager.claude_code_machine();
    let home = harness.manager.home(&machine);
    let older = install_by_hand(&harness.manager.artifacts(&machine), OLDER).await;
    let id = waiting_login(&backend, &master, LOGIN, new_entry).await;
    let LoginState::Completed {
        provider_id,
        credential_id: account,
    } = pasted(&backend, &master, &id, "first#state").await
    else {
        panic!("the sign-in did not complete");
    };
    let provider = provider_id.as_str().to_owned();
    let accounts_path = format!("/api/providers/{provider}/accounts");
    let directory = format!("{home}/accounts/{account}");
    let second_conversation = "7c1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a02";
    for conversation in [CONVERSATION, second_conversation] {
        create(&backend, &master, conversation).await;
        choose(&backend, &master, conversation, &provider, "claude-opus-4-8").await;
    }
    let mut first = Socket::connect(&backend, &master, CONVERSATION).await;
    first.open().await;
    first.chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a01", "hello").await;
    let mut second = Socket::connect(&backend, &master, second_conversation).await;
    second.open().await;

    // Signing in again: the kept process is asked to end, and takes its time.
    let hold = format!("{home}/hold-term");
    let held = format!("{home}/term-held");
    let release = format!("{home}/release-term");
    std::fs::write(&hold, "").unwrap();
    let id = waiting_login(&backend, &master, &format!("{accounts_path}/login"), json!({})).await;
    let code_path = format!("{LOGIN}/{id}/code");
    let pasted_code = backend.post(&code_path, Some(&master), json!({ "code": "first#again" })).await;
    assert_eq!(pasted_code.status, StatusCode::OK);
    eventually("the account's process is asked to end", || async {
        std::path::Path::new(&held).exists()
    })
    .await;
    // A request meanwhile waits for the change, then starts in the account's
    // new directory, with the new sign-in.
    let (answered, ()) = tokio::join!(
        second.chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a11", "meanwhile"),
        async {
            eventually("the request waits for the change", || {
                server_holds(&backend, &master, "a start of a Claude Code account waits for a change of the account")
            })
            .await;
            std::fs::write(&release, "").unwrap();
        }
    );
    drop(answered);
    let LoginState::Completed { credential_id, .. } = awaited(&backend, &master, &id, ended).await else {
        panic!("signing in again did not complete");
    };
    assert_eq!(credential_id, account);
    let said = last_text(&transcript(&backend, &master, second_conversation).await.blocks);
    let own = own_of(&said);
    assert_eq!(said, answer(&older, &home, "access-first-again", &account, &own));
    let processes = std::fs::read_to_string(format!("{home}/claude-processes.log")).unwrap();
    let lines: Vec<String> = processes.lines().map(str::to_owned).collect();
    assert_eq!(lines[1..], ["ended access-first-state".to_owned(), format!("started access-first-again {own}")]);

    // A login cancelled while it waits for the account's processes ends at
    // once, and the account keeps its directory.
    let id = waiting_login(&backend, &master, &format!("{accounts_path}/login"), json!({})).await;
    let code_path = format!("{LOGIN}/{id}/code");
    let pasted_code = backend.post(&code_path, Some(&master), json!({ "code": "first#third" })).await;
    assert_eq!(pasted_code.status, StatusCode::OK);
    eventually("the account's process is asked to end", || async {
        std::path::Path::new(&held).exists()
    })
    .await;
    let cancelled = backend.delete(&format!("{LOGIN}/{id}"), &master).await;
    assert_eq!(cancelled.status, StatusCode::NO_CONTENT);
    // The cancel answered while the account's process still ends: it did
    // not wait for it, which would have taken until its kill.
    let pid: i32 = std::fs::read_to_string(&held).unwrap().trim().parse().unwrap();
    let pid = rustix::process::Pid::from_raw(pid).unwrap();
    assert!(
        rustix::process::test_kill_process(pid).is_ok(),
        "the cancel waited for the account's process"
    );
    assert!(matches!(login_state(&backend, &master, &id).await, LoginState::Cancelled));
    // The cancelled login's directory goes while the account's process
    // still ends: the login did not wait to replace the account's.
    eventually("the cancelled login's directory goes", || async {
        login_directories(&home)
            .iter()
            .all(|(login, _)| !std::path::Path::new(login).exists())
    })
    .await;
    std::fs::remove_file(&hold).unwrap();
    std::fs::write(&release, "").unwrap();
    let signed_in = std::fs::read_to_string(format!("{directory}/.credentials.json")).unwrap();
    assert!(signed_in.contains("access-first-again"), "{signed_in}");

    // Removing the account closes its process and deletes its directory.
    first.chat("5e1d2e4f-8f3a-4c1e-9d2b-7a1c2e3f4a21", "once more").await;
    let removed = backend.delete(&format!("{accounts_path}/{account}"), &master).await;
    assert_eq!(removed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&removed.body));
    assert!(!std::path::Path::new(&directory).exists());
    let processes = std::fs::read_to_string(format!("{home}/claude-processes.log")).unwrap();
    assert_eq!(processes.lines().filter(|line| line.starts_with("ended")).count(), 3, "{processes}");
    drop((first, second));
    backend.close().await;
    log.close().await;
}

/// The handover of an earlier release's sign-in puts each file in place
/// whole, the credentials last (`crates/provider-claude-code/src/account.rs`): a
/// handover that cannot put the account's file in place leaves the
/// directory without a sign-in and the record holding it, and the next
/// start hands it over.
// A second or two: the machine boots, a sign-in is cancelled, and a CLI
// process starts.
#[tokio::test]
async fn a_handover_that_fails_leaves_no_sign_in_and_keeps_the_record() {
    let cache = tempfile::tempdir().unwrap();
    let (distribution, catalog) = vendors().await;
    let older = install_by_hand(cache.path(), OLDER).await;
    let kept = Kept::start(&distribution, &catalog, cache.path()).await;
    // A cancelled sign-in boots the machine.
    let accounts_path = format!("/api/providers/{}/accounts", kept.provider);
    let id = waiting_login(&kept.backend, &kept.master, &format!("{accounts_path}/login"), json!({})).await;
    let cancelled = kept.backend.delete(&format!("{LOGIN}/{id}"), &kept.master).await;
    assert_eq!(cancelled.status, StatusCode::NO_CONTENT);
    let accounts = kept.backend.get(&accounts_path, Some(&kept.master)).await.json::<Accounts>();
    let account = accounts.active.unwrap();
    let home = kept.harness.manager.home(&kept.harness.manager.claude_code_machine());
    let directory = format!("{home}/accounts/{account}");
    // The account's file cannot be put in place: its name is taken by a
    // directory that is not empty.
    let blocked = format!("{directory}/.claude.json");
    std::fs::create_dir_all(format!("{blocked}/inside")).unwrap();
    let said = kept.said(1).await;
    assert!(said.starts_with("failed: "), "{said}");
    assert!(!std::path::Path::new(&format!("{directory}/.credentials.json")).exists());
    let secret = || -> Option<Vec<u8>> {
        kept.harness
            .control_database()
            .query_row("SELECT secret FROM provider_credentials", [], |row| row.get(0))
            .unwrap()
    };
    assert!(secret().is_some());
    std::fs::remove_dir_all(&blocked).unwrap();
    let said = kept.said(2).await;
    assert!(said.starts_with(&format!("cli={older} token=access-kept ")), "{said}");
    assert_eq!(secret(), None);
    kept.close().await;
}
