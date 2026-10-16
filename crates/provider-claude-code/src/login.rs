//! The Claude sign-in (`claude-code.md` § Accounts and sign-in): Demi runs
//! the CLI's own login on the Claude Code machine, in a new configuration
//! directory, shows the user the link the CLI prints, and writes the code
//! the user pastes to the CLI's input. When the CLI exits with success, Demi
//! reads the account it signed in, its ID and email, and nothing of its
//! tokens, and the directory becomes the account's; when the login fails or
//! is stopped, the directory goes. A code the CLI refuses leaves it
//! waiting, and the user sees its words and pastes another. Demi never
//! writes Claude's OAuth flow itself, and never logs the code.

use std::sync::Arc;

use bytes::Bytes;
use demi_host_interface::{Process, ProcessControl, ProcessEnd};
use demi_provider_common::Secret;
use demi_provider_common::credentials::{
    AccountKit, AccountsCapability, AccountsError, CredentialPool, LoginError, LoginIo, LoginKind,
    NewAccount,
    credential_id_for, decode_secret, find_by_identity,
};
use demi_shared_types::{LoginPending, LoginStage, RefusedCode, StreamKind};
use futures_util::future::BoxFuture;
use futures_util::{FutureExt as _, StreamExt as _};
use reqwest::Url;
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::account::{identity_key, label};
use crate::cli;
use crate::files::{CONFIG, ConfigFile};
use crate::live::{Handle, Tail, exit_message};
use crate::placement::{
    AccountMachine, AccountWork, CliWork, ConfigDir, Placed, Placement, StartStage,
};

/// What the sign-in shows the user while it runs.
enum Shown {
    /// What the sign-in waits for before the CLI prints its link.
    Stage(LoginStage),
    /// The link the CLI printed.
    Link(String),
    /// The CLI refused a code the user pasted, and waits for another.
    CodeRefused(RefusedCode),
}

/// What the `claude-code` family adds to the account operations: the CLI's
/// own sign-in, on the machine account work runs on. A kit without a
/// machine, as of a provider built for inference, cannot sign in.
pub(crate) struct ClaudeKit {
    pub(crate) machine: Option<Arc<dyn AccountMachine>>,
    /// The entry the sign-in is for, or the login that is to make it.
    pub(crate) entry: String,
    /// The entry's accounts, which name the directory a sign-in's account
    /// takes.
    pub(crate) pool: Arc<dyn CredentialPool>,
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
        Some(Box::pin(sign_in(
            machine,
            self.entry.clone(),
            self.pool.clone(),
            io,
        )))
    }

    /// The account's directory on the machine goes before its record, so
    /// no sign-in outlives the account in Demi.
    fn forget<'a>(&'a self, id: &'a str) -> BoxFuture<'a, Result<(), AccountsError>> {
        Box::pin(async move {
            let machine = self.machine.clone().ok_or_else(|| {
                AccountsError::Unavailable("Claude Code's machine is not available".into())
            })?;
            machine
                .remove_account(id.to_owned())
                .await
                .map_err(|error| AccountsError::Unavailable(error.0))
        })
    }
}

/// Runs the CLI's sign-in on `machine` and relays it: what the start waits
/// for goes to `io.stage`, the link the CLI prints to `io.pending`, and the
/// codes the user pastes go to the CLI. It ends with the account the CLI
/// signed in, or once the sign-in failed or was stopped, after the
/// sign-in's directory went.
async fn sign_in(
    machine: Arc<dyn AccountMachine>,
    entry: String,
    pool: Arc<dyn CredentialPool>,
    io: LoginIo<'_>,
) -> Result<NewAccount, LoginError> {
    let LoginIo {
        pending,
        stage,
        codes,
        stop,
    } = io;
    let (shown_to, mut link) = mpsc::unbounded_channel();
    let (answer, answered) = oneshot::channel();
    let work: AccountWork = Box::new(move |placement, abandoned| {
        Box::pin(async move {
            let signed_in =
                sign_in_at(&*placement, &*pool, shown_to, codes, stop, abandoned).await;
            // A login that gave up waiting has stopped the sign-in already.
            let _ = answer.send(signed_in);
        })
    });
    let ran = machine.run(entry, work);
    tokio::pin!(ran);
    let mut link_url = None;
    let mut show = |next: Shown| match next {
        Shown::Stage(reached) => stage(reached),
        Shown::Link(url) => {
            pending(LoginPending {
                verification_url: url.clone(),
                user_code: None,
                code_error: None,
            });
            link_url = Some(url);
        }
        Shown::CodeRefused(refused) => {
            // The CLI reads codes only after it printed its link.
            if let Some(url) = &link_url {
                pending(LoginPending {
                    verification_url: url.clone(),
                    user_code: None,
                    code_error: Some(refused),
                });
            }
        }
    };
    let mut shown = true;
    let ran = loop {
        tokio::select! {
            biased;
            next = link.recv(), if shown => match next {
                Some(next) => show(next),
                None => shown = false,
            },
            ran = &mut ran => break ran,
        }
    };
    // What the sign-in reported before it ended is shown first, in its
    // order, so that its outcome names the stage it reached.
    while let Ok(next) = link.try_recv() {
        show(next);
    }
    ran.map_err(|error| LoginError::Failed(error.0))?;
    answered.await.unwrap_or_else(|_| {
        Err(LoginError::Failed(
            "The Claude sign-in ended without an answer".into(),
        ))
    })
}

/// The sign-in on the machine `placement` stands for, whose account takes
/// its id among `pool`'s. It ends early when `stop` fires or nobody waits
/// for it any more (`abandoned`), and removes its directory however it ends
/// but with success, which makes the directory the account's.
async fn sign_in_at(
    placement: &dyn Placement,
    pool: &dyn CredentialPool,
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
    // The login is gone once it stopped waiting, and a stage nobody shows
    // matters to nobody.
    let _work = placement.work();
    let stages = {
        let shown = shown.clone();
        move |reached: StartStage| {
            let _ = shown.send(Shown::Stage(match reached {
                StartStage::Machine => LoginStage::WaitingForMachine,
                StartStage::Install => LoginStage::Installing,
                StartStage::Process => LoginStage::Starting,
            }));
        }
    };
    // A start given up removes what it made.
    let Placed {
        process, config, ..
    } = tokio::select! {
        placed = placement.start(CliWork::Login, Arc::new(cli::login_request), &stages) => {
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
    // The process is held apart from what stops, and closed, SIGTERM
    // first, unless it ended by itself.
    let mut handle = Handle::of(control, exit.clone(), None);
    let relayed = tokio::select! {
        relayed = relay(output, handle.control(), shown, codes) => relayed,
        () = stopped() => Err(LoginError::Stopped),
    };
    let ended = match relayed {
        Ok(stderr) => tokio::select! {
            () = handle.ended() => match exit.clone().await {
                ProcessEnd::Exited(0) => Ok(()),
                end => Err(LoginError::Failed(if stderr.is_empty() {
                    exit_message(&end)
                } else {
                    stderr
                })),
            },
            () = stopped() => Err(LoginError::Stopped),
        },
        Err(error) => Err(error),
    };
    handle.close().await;
    if let Err(error) = ended {
        config.remove().await;
        return Err(error);
    }
    // The adoption waits for the account's processes, and stops with the
    // login all the same; one given up removes the sign-in's directory.
    tokio::select! {
        signed_in = signed_in(config, pool) => signed_in,
        () = stopped() => Err(LoginError::Stopped),
    }
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
    // The CLI judges the codes in the order it reads them, one refusal
    // each, so the nth refusal is of the nth code.
    let mut refused = 0;
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
                                refused += 1;
                                // As above.
                                let _ = shown.send(Shown::CodeRefused(RefusedCode {
                                    code: refused,
                                    message: line,
                                }));
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

/// The account the CLI's sign-in wrote into its directory `config`, which
/// becomes the account's directory: its ID and email, read from the CLI's
/// `.claude.json`, and nothing of its tokens. The account's id is the one
/// it has among `pool`'s accounts, or the one a new account gets.
async fn signed_in(
    config: Box<dyn ConfigDir>,
    pool: &dyn CredentialPool,
) -> Result<NewAccount, LoginError> {
    let read = async {
        let bytes = config.read(CONFIG).await.map_err(|error| {
            LoginError::Failed(format!("Claude Code wrote no {CONFIG}: {error}"))
        })?;
        let text = String::from_utf8(bytes.to_vec())
            .map_err(|_| LoginError::Failed(format!("Claude Code's {CONFIG} is not text")))?;
        decode_secret::<ConfigFile>(&text).map_err(|error| {
            LoginError::Failed(format!("Claude Code's {CONFIG} cannot be read: {error}"))
        })
    };
    let file = match read.await {
        Ok(file) => file,
        Err(error) => {
            config.remove().await;
            return Err(error);
        }
    };
    let account = file.oauth_account;
    let label = label(&account.account_uuid, account.email_address.as_deref());
    let key = identity_key(&account.account_uuid);
    let existing = find_by_identity(pool, &key).await;
    let id = match existing {
        Ok(Some(existing)) => existing.id,
        Ok(None) => credential_id_for(Some(&key), &label.label),
        Err(error) => {
            config.remove().await;
            return Err(LoginError::Failed(error.to_string()));
        }
    };
    config.adopt(&id).await.map_err(|error| {
        LoginError::Failed(format!(
            "Claude Code's sign-in could not become the account's: {error}"
        ))
    })?;
    Ok(NewAccount {
        label,
        secret: None,
    })
}
