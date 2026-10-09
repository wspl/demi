//! The Claude Code CLI through the backend (`claude-code.md` § Accounts and
//! sign-in, § Where it runs, § What the user sees, § Acceptance): a sign-in
//! runs the CLI's own login on the user's Cloud, relays its link, hands it
//! the pasted code, shows the CLI's words for a code it refuses, and keeps
//! the account it wrote, replacing the tokens of one signed in again, in a
//! directory that goes however the sign-in ends; a sign-in whose Cloud cannot install the CLI
//! says why, with the version, and so does a request; the settings read the
//! newest version and the Cloud's versions; a Cloud with an older CLI
//! answers with it; a conversation on a paired device infers through the
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

use std::time::Duration;

use demi_command_protocol::{ArtifactForm, PackageArtifact};
use demi_runner_command_packages::cache::{ArtifactCache, SILENT, Wanted};
use demi_runner_command_packages::{ArtifactResolver, ArtifactSource, RuntimeError};
use tokio_util::sync::CancellationToken;

use demi_provider_common::testing::{MockResponse, MockVendor};
use demi_shared_types::Block;
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::providers::{
    Accounts, CliInstall, LoginState, NewestVersion, ProviderCli, Providers,
};
use reqwest::StatusCode;
use serde_json::{Value, json};

use crate::accounts::{awaited, ended, login_state, start_login};
use crate::conversations::{Socket, choose, create, last_text, on_device, transcript};
use crate::support::{Harness, Session, TestBackend};

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
/// as the CLI writes them. Its other runs read
/// the access token from descriptor 3, answer the message the session they
/// resume ends with, answer `initialize`, answer each user message with the
/// executable, the token, how many environment variables hold it, the
/// directory they run in, their configuration directory and its mode, and
/// record their start and their end.
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
token=$(cat <&3)
echo "started $token $CLAUDE_CONFIG_DIR" >> "$log"
trap 'echo "ended $token" >> "$log"; exit 0' TERM
answer() {
  printf '{"type":"assistant","message":{"content":[{"type":"text","text":"cli=%s token=%s environment=%s cwd=%s config=%s mode=%s"}]}}\n' \
    "$0" "$token" "$(env | grep -c "$token")" "$(pwd)" "$CLAUDE_CONFIG_DIR" "$(ls -ld "$CLAUDE_CONFIG_DIR" | cut -c1-10)"
  printf '{"type":"result","usage":{"input_tokens":3,"output_tokens":2}}\n'
}
# The session it resumes ends with the message to answer.
answer
while IFS= read -r line; do
  case "$line" in
    '{"type":"control_request"'*'"subtype":"initialize"'*)
      id=$(printf '%s\n' "$line" | sed 's/^{"type":"control_request","request_id":"\([^"]*\)".*/\1/')
      printf '{"type":"control_response","response":{"subtype":"success","request_id":"%s"}}\n' "$id"
      ;;
    '{"type":"user"'*) answer ;;
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

/// Installs `version` of the scripted CLI in `cache`, a runner's artifact
/// cache, as the runner installs a download of it, and answers the
/// executable's path.
async fn install_by_hand(cache: &std::path::Path, version: &str) -> String {
    let source = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(source.path(), SCRIPTED_CLI).unwrap();
    let cache = ArtifactCache::new(cache.to_owned(), None, None)
        .await
        .unwrap();
    let wanted = Wanted {
        package: demi_command_package_claude_code_protocol::PACKAGE,
        name: "Claude Code",
        version,
        artifact: PackageArtifact {
            sha256: format!("{:x}", <sha2::Sha256 as sha2::Digest>::digest(SCRIPTED_CLI)),
            size: SCRIPTED_CLI.len() as u64,
        },
        form: &ArtifactForm::File,
    };
    let resolver = Local(source.path().to_owned());
    let path = cache
        .install(&wanted, &resolver, SILENT, &CancellationToken::new())
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

/// The user's one Cloud.
fn the_cloud(harness: &Harness) -> String {
    let devices = harness.manager.devices();
    let [cloud] = devices.as_slice() else {
        panic!("the user has one Cloud: {devices:?}");
    };
    cloud.clone()
}

/// The directories the CLI's logins ran in on the Cloud whose home is
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
        StatusCode::NO_CONTENT,
        "{}",
        String::from_utf8_lossy(&answer.body)
    );
    awaited(backend, master, id, ended).await
}

/// Pastes `code`, which the CLI refuses, into the master's login `id`, and
/// answers the words the login then shows while it waits.
async fn refused_code(backend: &TestBackend, master: &Session, id: &str, code: &str) -> String {
    let answer = backend
        .post(
            &format!("{LOGIN}/{id}/code"),
            Some(master),
            json!({ "code": code }),
        )
        .await;
    assert_eq!(answer.status, StatusCode::NO_CONTENT);
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
        ..
    } = refused
    else {
        panic!("the login did not wait for another code: {refused:?}");
    };
    words
}

/// What the scripted CLI answers a message with, in a process of `token`'s
/// whose configuration directory is `config`.
fn answer(cli: &str, home: &str, token: &str, config: &str) -> String {
    format!(
        "cli={cli} token={token} environment=0 cwd={home}/.demi/claude/run config={config} mode=drwx------"
    )
}

/// The configuration directory an answer of the scripted CLI names.
fn config_of(answer: &str) -> String {
    let start = answer.find("config=").expect("the answer names its directory") + 7;
    answer[start..]
        .split(' ')
        .next()
        .unwrap()
        .to_owned()
}

// About two seconds: the Cloud boots and installs the `demi.claude-code`
// package, which runs the CLI's logins and three of its processes.
#[tokio::test]
async fn a_sign_in_on_the_cloud_takes_the_pasted_code_and_each_account_infers_in_a_directory_of_its_own()
 {
    let (distribution, catalog) = vendors().await;
    let harness = Harness::new()
        .with_claude_package()
        .with_claude_releases(distribution.url("/releases"))
        .with_models_dev(catalog.url("/api.json"));
    let (backend, master) = harness.start_set_up().await;

    // A sign-in runs Demi's CLI on the user's Cloud, which has none; the
    // install fails, and the sign-in says why, with the version.
    let id = start_login(
        &backend,
        &master,
        LOGIN,
        json!({ "providerType": "claude-code", "label": "Claude" }),
    )
    .await;
    let LoginState::Failed { message } = awaited(&backend, &master, &id, ended).await else {
        panic!("the sign-in without a CLI did not fail");
    };
    let reason = format!("Claude Code {NEWEST} could not be installed: invalid release record");
    assert!(message.starts_with(&reason), "{message}");
    let providers = backend.get("/api/providers", Some(&master)).await;
    assert!(providers.json::<Providers>().providers.is_empty());
    let cloud = the_cloud(&harness);
    let home = harness.manager.home(&cloud);
    let older = install_by_hand(&harness.manager.artifacts(&cloud), OLDER).await;

    // The sign-in shows the link the CLI printed and hands the CLI the code
    // the user pastes; the account the CLI wrote is stored, and the CLI's
    // directory, private to the Cloud's user, is gone.
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
    assert_eq!(logins[0].1, "drwx------");
    assert!(!std::path::Path::new(&logins[0].0).exists());
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
    assert_eq!(
        (account.label.as_str(), account.detail.as_deref()),
        ("first@example.test", Some("max"))
    );
    assert_eq!(accounts.active.as_ref(), Some(&first));
    let read = cli(&backend, &master, &provider).await;
    assert_eq!(
        read.newest,
        NewestVersion::Read {
            version: NEWEST.into()
        }
    );
    assert_eq!(read.machines[0].versions, Some(vec![OLDER.to_owned()]));

    // A request of a conversation whose files are on a paired laptop infers
    // through the Cloud's CLI, which reads the account's token from a
    // descriptor and has a private directory of its own; nothing of it
    // reaches the laptop.
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
    let first_config = config_of(&first_answer);
    assert!(
        first_config.starts_with("/tmp/demi-claude-"),
        "{first_answer}"
    );
    assert_eq!(
        first_answer,
        answer(&older, &home, "access-first-state", &first_config)
    );
    assert!(!laptop.runner.home_dir().join(".demi/claude").exists());

    // Another account signs in beside it: a code the CLI refuses keeps the
    // login waiting, with the CLI's words, until the user pastes another.
    // Signing in to the first account again replaces its tokens and keeps
    // its ID and the entry's choice. No login leaves its directory.
    let accounts_path = format!("/api/providers/{provider}/accounts");
    let id = waiting_login(&backend, &master, &format!("{accounts_path}/login"), json!({})).await;
    assert_eq!(
        refused_code(&backend, &master, &id, "invalid").await,
        "Invalid code. Please make sure the full code was copied."
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
    let listed = backend
        .get(&accounts_path, Some(&master))
        .await
        .json::<Accounts>();
    assert_eq!(listed.accounts.len(), 2);
    assert_eq!(listed.active.as_ref(), Some(&first));

    // After the other account is selected, the next request closes the
    // process, whose directory goes, and starts one with that account's
    // token in a new directory.
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
    let second_config = config_of(&second_answer);
    assert_ne!(second_config, first_config);
    assert_eq!(
        second_answer,
        answer(&older, &home, "access-second-state", &second_config)
    );
    assert!(!std::path::Path::new(&first_config).exists());

    // Back on the first account, a new process has its new token.
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
    let third_config = config_of(&third_answer);
    assert_eq!(
        third_answer,
        answer(&older, &home, "access-first-again", &third_config)
    );
    let processes = std::fs::read_to_string(format!("{home}/claude-processes.log")).unwrap();
    let expected = [
        format!("started access-first-state {first_config}"),
        "ended access-first-state".to_owned(),
        format!("started access-second-state {second_config}"),
        "ended access-second-state".to_owned(),
        format!("started access-first-again {third_config}"),
    ];
    assert_eq!(processes.lines().collect::<Vec<_>>(), expected);
    // The distribution was read once: its answer is believed.
    assert_eq!(distribution.requests().len(), 2);
    backend.close().await;
    assert!(!std::path::Path::new(&third_config).exists());
}

// About two seconds, most of it the one login that expires.
#[tokio::test]
async fn a_sign_in_that_fails_is_cancelled_or_expires_stores_nothing_and_leaves_no_directory() {
    let (distribution, catalog) = vendors().await;
    let mut harness = Harness::new()
        .with_claude_package()
        .with_claude_releases(distribution.url("/releases"))
        .with_models_dev(catalog.url("/api.json"));
    harness.logins.code_lifetime = Duration::from_secs(2);
    let (backend, master) = harness.start_set_up().await;
    let new_entry = json!({ "providerType": "claude-code" });
    // The first sign-in wakes the Cloud, which has no CLI to run.
    let id = start_login(&backend, &master, LOGIN, new_entry.clone()).await;
    assert!(matches!(
        awaited(&backend, &master, &id, ended).await,
        LoginState::Failed { .. }
    ));
    let cloud = the_cloud(&harness);
    let home = harness.manager.home(&cloud);
    install_by_hand(&harness.manager.artifacts(&cloud), OLDER).await;

    // The CLI's own words of failure are what the user sees.
    let id = waiting_login(&backend, &master, LOGIN, new_entry.clone()).await;
    assert_eq!(
        pasted(&backend, &master, &id, "refused#state").await,
        LoginState::Failed {
            message: "Login failed: the code was refused".into()
        }
    );

    // A cancelled sign-in has ended, with its directory gone, once the
    // cancel answers.
    let id = waiting_login(&backend, &master, LOGIN, new_entry.clone()).await;
    let cancelled = backend.delete(&format!("{LOGIN}/{id}"), &master).await;
    assert_eq!(cancelled.status, StatusCode::NO_CONTENT);
    assert_eq!(
        login_state(&backend, &master, &id).await,
        LoginState::Failed {
            message: "The login was cancelled".into()
        }
    );
    let logins = login_directories(&home);
    assert_eq!(logins.len(), 2, "{logins:?}");
    assert!(!std::path::Path::new(&logins[1].0).exists());

    // A sign-in nobody finishes ends at its expiry.
    let id = waiting_login(&backend, &master, LOGIN, new_entry).await;
    assert_eq!(
        awaited(&backend, &master, &id, ended).await,
        LoginState::Failed {
            message: "The login expired".into()
        }
    );
    let logins = login_directories(&home);
    assert_eq!(logins.len(), 3, "{logins:?}");
    assert!(
        logins
            .iter()
            .all(|(directory, _)| !std::path::Path::new(directory).exists())
    );
    let providers = backend.get("/api/providers", Some(&master)).await;
    assert!(providers.json::<Providers>().providers.is_empty());
    backend.close().await;
}
