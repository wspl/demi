//! The Claude sign-in (`claude-code.md` § Accounts and sign-in): Demi runs
//! the CLI's own login on the acting user's Cloud, in a configuration
//! directory of its own, shows the user the link the CLI prints, and writes
//! the code the user pastes to the CLI's input. When the CLI exits with
//! success, Demi reads the tokens and the account it wrote, and the
//! directory goes, as it does when the login fails or is stopped. A code the
//! CLI refuses leaves it waiting, and the user sees its words and pastes
//! another. Demi never writes Claude's OAuth flow itself, and never logs the
//! code.

use std::sync::Arc;

use bytes::Bytes;
use demi_host_interface::{Process, ProcessControl, ProcessEnd, Signal};
use demi_provider_common::Secret;
use demi_provider_common::credentials::{
    AccountKit, AccountsCapability, LoginError, LoginIo, LoginKind, NewAccount, SecretDocument,
    decode_secret,
};
use demi_shared_types::{LoginPending, StreamKind, Timestamp};
use futures_util::future::BoxFuture;
use futures_util::{FutureExt as _, StreamExt as _};
use reqwest::Url;
use serde::Deserialize;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::account::ClaudeSecret;
use crate::cli;
use crate::live::{Tail, exit_message};
use crate::placement::{AccountMachine, AccountWork, Placed, Placement};

/// The file the CLI writes its tokens to in its configuration directory.
const CREDENTIALS: &str = ".credentials.json";
/// The file the CLI writes its account to there.
const CONFIG: &str = ".claude.json";

/// What the sign-in shows the user while it runs.
enum Shown {
    /// The link the CLI printed.
    Link(String),
    /// The CLI refused the code the user pasted, in these words, and waits
    /// for another.
    CodeRefused(String),
}

/// What the `claude-code` family adds to the account operations: the CLI's
/// own sign-in, on the machine account work runs on. A kit without a
/// machine, as of a provider built for inference, cannot sign in.
pub(crate) struct ClaudeKit {
    pub(crate) machine: Option<Arc<dyn AccountMachine>>,
    /// The entry the sign-in is for, or the login that is to make it.
    pub(crate) entry: String,
}

impl AccountKit for ClaudeKit {
    fn capability(&self) -> AccountsCapability {
        AccountsCapability {
            login: self.machine.is_some().then_some(LoginKind::PastedCode),
        }
    }

    fn login<'a>(
        &'a self,
        io: LoginIo<'a>,
    ) -> Option<BoxFuture<'a, Result<NewAccount, LoginError>>> {
        let machine = self.machine.clone()?;
        Some(Box::pin(sign_in(machine, self.entry.clone(), io)))
    }

}

/// Runs the CLI's sign-in on `machine` and relays it: the link the CLI
/// prints goes to `io.pending`, and the codes the user pastes go to the
/// CLI. It ends with the account the CLI signed in, or once the sign-in
/// failed or was stopped, after the sign-in's directory went.
async fn sign_in(
    machine: Arc<dyn AccountMachine>,
    entry: String,
    io: LoginIo<'_>,
) -> Result<NewAccount, LoginError> {
    let LoginIo {
        pending,
        codes,
        stop,
    } = io;
    let (shown_to, mut link) = mpsc::unbounded_channel();
    let (answer, answered) = oneshot::channel();
    let work: AccountWork = Box::new(move |placement, abandoned| {
        Box::pin(async move {
            let signed_in = sign_in_at(&*placement, shown_to, codes, stop, abandoned).await;
            // A login that gave up waiting has stopped the sign-in already.
            let _ = answer.send(signed_in);
        })
    });
    let ran = machine.run(entry, work);
    tokio::pin!(ran);
    let mut shown = true;
    let mut link_url = None;
    loop {
        tokio::select! {
            next = link.recv(), if shown => match next {
                Some(Shown::Link(url)) => {
                    pending(LoginPending {
                        verification_url: url.clone(),
                        user_code: None,
                        code_error: None,
                    });
                    link_url = Some(url);
                }
                Some(Shown::CodeRefused(words)) => {
                    // The CLI reads codes only after it printed its link.
                    if let Some(url) = &link_url {
                        pending(LoginPending {
                            verification_url: url.clone(),
                            user_code: None,
                            code_error: Some(words),
                        });
                    }
                }
                None => shown = false,
            },
            ran = &mut ran => {
                ran.map_err(|error| LoginError::Failed(error.0))?;
                break;
            }
        }
    }
    answered.await.unwrap_or_else(|_| {
        Err(LoginError::Failed(
            "The Claude sign-in ended without an answer".into(),
        ))
    })
}

/// The sign-in on the machine `placement` stands for. It ends early when
/// `stop` fires or nobody waits for it any more (`abandoned`), and removes
/// its directory however it ends.
async fn sign_in_at(
    placement: &dyn Placement,
    shown: mpsc::UnboundedSender<Shown>,
    codes: mpsc::UnboundedReceiver<Secret>,
    stop: CancellationToken,
    abandoned: CancellationToken,
) -> Result<NewAccount, LoginError> {
    let stopped = || async {
        tokio::select! {
            () = stop.cancelled() => {}
            () = abandoned.cancelled() => {}
        }
    };
    // A start given up removes what it made.
    let Placed { process, config } = tokio::select! {
        placed = placement.start(&cli::login_request) => {
            placed.map_err(|error| LoginError::Failed(error.0))?
        }
        () = stopped() => return Err(LoginError::Stopped),
    };
    let Process {
        output,
        control,
        exit,
    } = process;
    let exit = exit.shared();
    let relayed = tokio::select! {
        relayed = relay(output, &*control, shown, codes) => relayed,
        () = stopped() => Err(LoginError::Stopped),
    };
    let signed_in = match relayed {
        Ok(stderr) => match exit.clone().await {
            ProcessEnd::Exited(0) => signed_in(&*config).await,
            end => Err(LoginError::Failed(if stderr.is_empty() {
                exit_message(&end)
            } else {
                stderr
            })),
        },
        Err(error) => {
            // A process that already ended takes no signal; its end is
            // awaited either way.
            let _ = control.kill(Signal::Kill).await;
            exit.clone().await;
            Err(error)
        }
    };
    drop(control);
    config.remove().await;
    signed_in
}

/// Relays the sign-in until the CLI's output ends: the link it prints goes
/// to `shown`, and each code to its input, followed by a newline. A code
/// the CLI calls invalid goes to `shown` in the CLI's words, while the CLI
/// waits for another. Answers the tail of its standard error.
async fn relay(
    mut output: futures_util::stream::LocalBoxStream<'static, demi_host_interface::ProcessOutput>,
    control: &dyn ProcessControl,
    shown: mpsc::UnboundedSender<Shown>,
    mut codes: mpsc::UnboundedReceiver<Secret>,
) -> Result<String, LoginError> {
    let mut stdout = Vec::new();
    let mut stderr = Tail::default();
    let mut stderr_line = Vec::new();
    let mut linked = false;
    let mut coded = false;
    loop {
        tokio::select! {
            chunk = output.next() => {
                let Some(chunk) = chunk else {
                    return Ok(stderr.text());
                };
                match chunk.stream {
                    StreamKind::Stdout => {
                        stdout.extend_from_slice(&chunk.bytes);
                        while let Some(end) = stdout.iter().position(|byte| *byte == b'\n') {
                            let line: Vec<u8> = stdout.drain(..=end).collect();
                            if linked {
                                continue;
                            }
                            if let Some(url) = link(&String::from_utf8_lossy(&line)) {
                                linked = true;
                                // The login is gone once it stopped waiting.
                                let _ = shown.send(Shown::Link(url));
                            }
                        }
                    }
                    StreamKind::Stderr => {
                        stderr.push(&chunk.bytes);
                        stderr_line.extend_from_slice(&chunk.bytes);
                        while let Some(end) = stderr_line.iter().position(|byte| *byte == b'\n') {
                            let line: Vec<u8> = stderr_line.drain(..=end).collect();
                            let line = String::from_utf8_lossy(&line).trim().to_owned();
                            if line.starts_with("Invalid code") {
                                // As above.
                                let _ = shown.send(Shown::CodeRefused(line));
                            }
                        }
                    }
                }
            }
            code = codes.recv(), if !coded => {
                let Some(code) = code else {
                    coded = true;
                    continue;
                };
                let mut line = code.expose().as_bytes().to_vec();
                line.push(b'\n');
                control.write_stdin(Bytes::from(line)).await.map_err(|error| {
                    LoginError::Failed(format!("The code could not be given to Claude Code: {error}"))
                })?;
            }
        }
    }
}

/// The sign-in link a line of the CLI's output names: its first `https`
/// address, inside a terminal hyperlink when the CLI wrote one.
fn link(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let rest = &line[start..];
    let end = rest
        .find(|character: char| character.is_whitespace() || character.is_control())
        .unwrap_or(rest.len());
    let url = Url::parse(&rest[..end]).ok()?;
    (url.scheme() == "https").then(|| url.to_string())
}

/// The account the CLI's sign-in wrote into its directory.
async fn signed_in(config: &dyn crate::placement::ConfigDir) -> Result<NewAccount, LoginError> {
    let read = |name: &'static str| async move {
        let bytes = config.read(name).await.map_err(|error| {
            LoginError::Failed(format!("Claude Code wrote no {name}: {error}"))
        })?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| LoginError::Failed(format!("Claude Code's {name} is not text")))
    };
    let unreadable =
        |name: &str, error| LoginError::Failed(format!("Claude Code's {name} cannot be read: {error}"));
    let credentials: CredentialsFile = decode_secret(&read(CREDENTIALS).await?)
        .map_err(|error| unreadable(CREDENTIALS, error))?;
    let config: ConfigFile =
        decode_secret(&read(CONFIG).await?).map_err(|error| unreadable(CONFIG, error))?;
    let tokens = credentials.claude_ai_oauth;
    let expires_at = Timestamp::from_millisecond(tokens.expires_at).map_err(|_| {
        LoginError::Failed("Claude Code's sign-in names an expiry out of range".into())
    })?;
    let secret = ClaudeSecret {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
        expires_at,
        scopes: tokens.scopes,
        subscription_type: tokens.subscription_type,
        account_id: config.oauth_account.account_uuid,
        email: config.oauth_account.email_address,
    };
    Ok(NewAccount {
        label: secret.label(),
        secret: secret.encode(),
    })
}

/// The CLI's credentials file, of which Demi reads the sign-in's tokens.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CredentialsFile {
    claude_ai_oauth: SignedInTokens,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct SignedInTokens {
    access_token: Secret,
    refresh_token: Secret,
    /// Milliseconds since the Unix epoch.
    expires_at: i64,
    scopes: Vec<String>,
    #[serde(default)]
    subscription_type: Option<String>,
}

/// The CLI's configuration file, of which Demi reads the signed-in account.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ConfigFile {
    oauth_account: OauthAccount,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct OauthAccount {
    account_uuid: String,
    #[serde(default)]
    email_address: Option<String>,
}
