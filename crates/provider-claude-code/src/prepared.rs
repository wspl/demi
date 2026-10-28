//! Processes prepared ahead.
//!
//! When a session's next request is known but for its input, its process starts
//! before the request, resumes the history and waits, and the request takes it
//! as it would take a kept process.
//!
//! A task owns the process while it waits. It answers the CLI's MCP messages,
//! hands the process to the request that matches it, and closes it when the
//! runtime drops it or its seat among the machine's idle processes says so
//! (`crates/provider-claude-code/src/idle.rs`). A preparation that fails is
//! logged and forgotten.
//!
//! A request that finds no kept process waits for a new one. On a production
//! server a subagent's median first turn waited 3.0 s before its CLI was ready
//! and 5.7 s for its first word, against 1.4 s for a kept process. So the
//! process is started whenever the request it will serve is known but for its
//! input.
//!
//! - A preparation does everything a start does, so it reads the blobs of the
//!   media its history replays, after the page's open is answered: the rule
//!   that an open reads no blob holds for the open itself.
//! - It never gets in the way: it takes no admission the user's own commands
//!   wait for, and one that fails is logged and forgotten.

use std::rc::Rc;
use std::sync::Arc;

use demi_provider_common::{InferenceItem, InferenceRequest};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::Shared;
use crate::idle::{self, Closing, Idle, Seat};
use crate::live::{LiveCli, ProcessKey, Written};
use crate::placement::Placement;
use crate::timing::Took;

/// What the runtime asks of a prepared process's task.
enum Command {
    /// Hand the process over, once it is ready.
    Take(oneshot::Sender<LiveCli>),
    /// Close it.
    Close,
}

/// A runtime's process prepared for the request that `request` is but for
/// its input.
pub(crate) struct Prepared {
    key: ProcessKey,
    system_prompt: String,
    items: Arc<[InferenceItem]>,
    commands: oneshot::Sender<Command>,
    task: JoinHandle<()>,
}

impl Prepared {
    /// Starts preparing a process for `request` through `placement`, when its
    /// session ends with an answer.
    pub(crate) fn start(
        shared: &Arc<Shared>,
        placement: &Rc<dyn Placement>,
        request: InferenceRequest,
    ) -> Self {
        let (commands, received) = oneshot::channel();
        let key = ProcessKey::of(&request);
        let system_prompt = request.system_prompt.clone();
        let items = request.items.clone();
        let task = tokio::task::spawn_local(wait(
            shared.clone(),
            placement.clone(),
            request,
            received,
        ));
        Self {
            key,
            system_prompt,
            items,
            commands,
            task,
        }
    }

    /// Whether the process was prepared for `request`: the same session,
    /// model, effort and system prompt, and the history before its input is
    /// the one it was prepared with.
    pub(crate) fn serves(&self, request: &InferenceRequest) -> bool {
        self.settings_of(request)
            && request.items.len() > self.items.len()
            && request.items[..self.items.len()] == self.items[..]
    }

    /// Whether `request`, a request but for its input, is still the one the
    /// process was prepared for.
    pub(crate) fn prepared_for(&self, request: &InferenceRequest) -> bool {
        self.settings_of(request) && request.items[..] == self.items[..]
    }

    /// Whether `request` has the session, model, effort and system prompt
    /// the process was prepared with.
    fn settings_of(&self, request: &InferenceRequest) -> bool {
        self.key == ProcessKey::of(request) && self.system_prompt == request.system_prompt
    }

    /// The process, once it is ready; none when its preparation failed or
    /// it ended meanwhile.
    pub(crate) async fn take(self) -> Option<LiveCli> {
        let (given, taken) = oneshot::channel();
        self.commands.send(Command::Take(given)).ok()?;
        taken.await.ok()
    }

    /// Closes the process, or gives up its start, and waits until it is
    /// done.
    pub(crate) async fn close(self) {
        // A task that ended has nothing left to close.
        let _ = self.commands.send(Command::Close);
        // A task that panicked left nothing to wait for.
        let _ = self.task.await;
    }
}

/// The task of a prepared process: it writes the session, and when that ends
/// with an answer, starts the process as a request would, then answers what
/// the CLI asks while it waits, until a request takes it or it is closed.
///
/// A session that ends with input prepares nothing: the CLI would answer that
/// input at once, and the next request's own input after it. Its seat counts
/// the process idle from its start. Closing a process still starting gives its
/// start up, which closes what it started.
async fn wait(
    shared: Arc<Shared>,
    placement: Rc<dyn Placement>,
    request: InferenceRequest,
    mut commands: oneshot::Receiver<Command>,
) {
    let session = request.session_id.clone();
    let written = tokio::select! {
        written = Written::of(&shared, &request) => written,
        // A request that asks for the process meanwhile starts its own.
        _ = &mut commands => return,
    };
    let written = match written {
        Ok(written) if written.file.answered => written,
        Ok(_) => {
            tracing::debug!(
                event = "claude_code_prepared.unanswered",
                provider = %shared.id,
                node = session,
                "no Claude Code process is prepared for a history that ends with input"
            );
            return;
        }
        Err(failure) => {
            tracing::warn!(
                event = "claude_code_prepared.start_failed",
                provider = %shared.id,
                node = session,
                error = failure.message,
                "a Claude Code process prepared ahead did not start"
            );
            return;
        }
    };
    let seat = placement.idle().seat(&shared.id, shared.auth.account().ok(), Idle::Prepared);
    let closing = seat.closing();
    tokio::pin!(closing);
    let started = async {
        let mut live = LiveCli::start(&shared, &*placement, &request, written, Took::Prepared).await?;
        live.prepare(&request).await?;
        Ok::<_, demi_provider_common::ProviderFailure>(live)
    };
    tokio::pin!(started);
    // A request that asks for the process while it starts waits for it.
    let mut taker = None;
    let started = loop {
        tokio::select! {
            started = &mut started => break started,
            command = &mut commands, if taker.is_none() => match command {
                Ok(Command::Take(given)) => taker = Some(given),
                // Dropping the start gives it up and closes what it started.
                Ok(Command::Close) | Err(_) => return,
            },
            // A request that stopped waiting took the process from its
            // runtime, so nobody else will.
            () = taken_by_nobody(&mut taker), if taker.is_some() => return,
            closed = &mut closing, if taker.is_none() => {
                if closed == Closing::Unused {
                    idle::log_unused(&shared, &session, Idle::Prepared);
                }
                return;
            }
        }
    };
    let mut live = match started {
        Ok(live) => live,
        Err(failure) => {
            tracing::warn!(
                event = "claude_code_prepared.start_failed",
                provider = %shared.id,
                node = session,
                error = failure.message,
                "a Claude Code process prepared ahead did not start"
            );
            return;
        }
    };
    if let Some(given) = taker {
        hand_over(live, given, seat).await;
        return;
    }
    loop {
        let waited = tokio::select! {
            biased;
            command = &mut commands => {
                match command {
                    Ok(Command::Take(given)) => hand_over(live, given, seat).await,
                    Ok(Command::Close) | Err(_) => live.close().await,
                }
                return;
            }
            closed = &mut closing => {
                if closed == Closing::Unused {
                    idle::log_unused(&shared, &session, Idle::Prepared);
                }
                live.close().await;
                return;
            }
            waited = live.wait() => waited,
        };
        if let Err(failure) = live.answer_waiting(waited).await {
            tracing::warn!(
                event = "claude_code_prepared.wait_failed",
                provider = %shared.id,
                node = session,
                error = failure.message,
                "a Claude Code process prepared ahead failed while it waited"
            );
            live.close().await;
            return;
        }
    }
}

/// Resolves once the request in `taker` stopped waiting for the process.
async fn taken_by_nobody(taker: &mut Option<oneshot::Sender<LiveCli>>) {
    match taker {
        Some(taker) => taker.closed().await,
        None => std::future::pending().await,
    }
}

/// Gives `live` to the request that asked for it; the process leaves its
/// machine's prepared ones. A request that stopped waiting meanwhile
/// leaves it to be closed.
async fn hand_over(live: LiveCli, given: oneshot::Sender<LiveCli>, seat: Seat) {
    drop(seat);
    if let Err(live) = given.send(live) {
        live.close().await;
    }
}
