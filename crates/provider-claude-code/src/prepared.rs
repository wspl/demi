//! Processes prepared ahead (`claude-code.md` § Preparing a process ahead):
//! when a session's next request is known but for its input, its process
//! starts before the request, resumes the history and waits, and the
//! request takes it as it would take a kept process. A task owns the
//! process while it waits: it answers the CLI's MCP handshake, hands the
//! process to the request that matches it, and closes it once it went
//! unused for ten minutes, when the runtime drops it, or when its machine
//! needs its seat. A preparation that fails is logged and forgotten.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::time::Duration;

use demi_provider_common::{InferenceItem, InferenceRequest};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;

use crate::Shared;
use crate::live::{LiveCli, ProcessKey};
use crate::placement::Placement;

/// How long a prepared process waits for its request before it is closed.
pub(crate) const UNUSED_FOR: Duration = Duration::from_secs(10 * 60);

/// How many prepared processes not yet used one machine holds.
const MOST_ON_A_MACHINE: usize = 2;

/// The processes prepared ahead on one machine and not yet used, of every
/// runtime that starts its processes there. It holds at most two: a third
/// preparation closes the oldest. It lives with its machine's placement on
/// the machine's user's shard.
#[derive(Default)]
pub struct PreparedProcesses {
    seats: RefCell<VecDeque<SeatRecord>>,
    /// How many preparations began on the machine, which also numbers them.
    begun: tokio::sync::watch::Sender<u64>,
}

struct SeatRecord {
    id: u64,
    /// The provider entry whose runtime prepared the process.
    entry: String,
    close: CancellationToken,
}

impl PreparedProcesses {
    /// A seat for a process `entry`'s runtime prepares; the oldest seat
    /// beyond the machine's two is told to close.
    fn seat(self: &Rc<Self>, entry: &str) -> Seat {
        let id = *self.begun.borrow();
        self.begun.send_replace(id + 1);
        let close = CancellationToken::new();
        let mut seats = self.seats.borrow_mut();
        seats.push_back(SeatRecord {
            id,
            entry: entry.to_owned(),
            close: close.clone(),
        });
        while seats.len() > MOST_ON_A_MACHINE {
            if let Some(oldest) = seats.pop_front() {
                oldest.close.cancel();
            }
        }
        Seat {
            id,
            set: Rc::downgrade(self),
            close,
        }
    }

    /// Closes every process prepared for the provider entry `entry`, as
    /// when the entry's active account changed and they hold the previous
    /// account's token.
    pub fn close_entry(&self, entry: &str) {
        self.seats.borrow_mut().retain(|seat| {
            let kept = seat.entry != entry;
            if !kept {
                seat.close.cancel();
            }
            kept
        });
    }

    /// Closes every process prepared on the machine.
    pub fn close_all(&self) {
        for seat in self.seats.borrow_mut().drain(..) {
            seat.close.cancel();
        }
    }

    /// Resolves once `count` preparations began on the machine since it was
    /// first used, for a check of what starts one.
    pub async fn until_begun(&self, count: u64) {
        let mut begun = self.begun.subscribe();
        // The sender lives in `self`, which this call borrows.
        let _ = begun.wait_for(|begun| *begun >= count).await;
    }

    /// How many prepared processes the machine holds unused.
    pub fn len(&self) -> usize {
        self.seats.borrow().len()
    }

    pub fn is_empty(&self) -> bool {
        self.seats.borrow().is_empty()
    }
}

/// A prepared process's place among its machine's; leaving it, when the
/// process is used or closed, frees it.
struct Seat {
    id: u64,
    set: Weak<PreparedProcesses>,
    close: CancellationToken,
}

impl Drop for Seat {
    fn drop(&mut self) {
        if let Some(set) = self.set.upgrade() {
            set.seats.borrow_mut().retain(|seat| seat.id != self.id);
        }
    }
}

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
    /// Starts preparing a process for `request`, whose history ends with an
    /// answer, through `placement`.
    pub(crate) fn start(
        shared: &Arc<Shared>,
        http: &reqwest::Client,
        placement: &Rc<dyn Placement>,
        request: InferenceRequest,
    ) -> Self {
        let (commands, received) = oneshot::channel();
        let seat = placement.prepared().seat(&shared.id);
        let key = ProcessKey::of(&request);
        let system_prompt = request.system_prompt.clone();
        let items = request.items.clone();
        let task = tokio::task::spawn_local(wait(
            shared.clone(),
            http.clone(),
            placement.clone(),
            request,
            received,
            seat,
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

/// The task of a prepared process: it starts the process as a request
/// would, then answers what the CLI asks while it waits, until a request
/// takes it or it is closed. Ten minutes after the preparation began, a
/// process no request took is closed. Closing a process still starting
/// gives its start up, which kills what it started.
async fn wait(
    shared: Arc<Shared>,
    http: reqwest::Client,
    placement: Rc<dyn Placement>,
    request: InferenceRequest,
    mut commands: oneshot::Receiver<Command>,
    seat: Seat,
) {
    let session = request.session_id.clone();
    let unused = tokio::time::sleep(UNUSED_FOR);
    tokio::pin!(unused);
    let started = async {
        let mut live = LiveCli::start(&shared, &http, &*placement, &request, None).await?;
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
                // Dropping the start gives it up and kills what it started.
                Ok(Command::Close) | Err(_) => return,
            },
            () = seat.close.cancelled() => return,
            () = &mut unused, if taker.is_none() => return,
        }
    };
    let mut live = match started {
        Ok(live) => live,
        Err(failure) => {
            tracing::warn!(
                provider = %shared.id,
                session,
                "a Claude Code process prepared ahead did not start: {}",
                failure.message
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
            () = seat.close.cancelled() => {
                live.close().await;
                return;
            }
            () = &mut unused => {
                tracing::info!(
                    provider = %shared.id,
                    session,
                    "a Claude Code process prepared ahead went unused and was closed"
                );
                live.close().await;
                return;
            }
            waited = live.wait() => waited,
        };
        if let Err(failure) = live.answer_waiting(waited).await {
            tracing::warn!(
                provider = %shared.id,
                session,
                "a Claude Code process prepared ahead failed while it waited: {}",
                failure.message
            );
            live.close().await;
            return;
        }
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
