//! How work running under the runner reaches the backend and the registration's
//! owner (`crates/runner-protocol/src/wire/mod.rs`,
//! `crates/runner/src/connection/kept.rs`):
//!
//! - a callback call registers where its events go;
//! - an artifact download asks where its artifact is;
//! - a service asks for its conversation numbers;
//! - a job makes its execution context live.
//!
//! The handle outlives each connection, since a job does: work that needs the
//! backend waits for the next connection while the runner is away, however long
//! it takes, and fails only once the runner ends. The registration's owner
//! serves the requests and routes the backend's answers through a [`Relay`].

use std::{collections::HashMap, io, sync::Arc, time::Duration};

use demi_command_protocol::{ArtifactLocation, ServiceSequence, host_target};
use demi_runner_command_packages::{ServiceBackend, RuntimeError};
use demi_runner_protocol::wire::{self, Inbound};
use futures_util::future::BoxFuture;
use tokio::{
    sync::{mpsc, oneshot, watch},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

use crate::commands::{
    contexts::{ExecutionContext, PackageLeases},
    rpc::CallEvent,
};

/// Requests waiting for the registration's owner; a sender waits for room.
const REQUESTS: usize = 64;
/// How long the backend has to answer which numbers a service may use. Where
/// an artifact is has no deadline of its own: the backend may fetch it first,
/// which takes as long as the server's disk or link does
/// (`crates/backend-runners/src/publication.rs`).
const NUMBERS_TIMEOUT: Duration = Duration::from_secs(15);

/// Why work that needs the backend did not reach it: the runner ends
/// (`crates/runner/src/connection/kept.rs`).
pub const UNREACHABLE: &str = "the backend is unreachable";

/// Whether the runner reaches its backend now.
#[derive(Clone)]
pub enum Reach {
    /// A connection serves, until `closed`.
    Connected(Live),
    /// The connection was lost; the runner waits for the next.
    Away,
    /// The runner ends, and its jobs with it, as when the backend refused
    /// it.
    Unreached,
}

/// The connection that serves now: its messages to the backend, and its
/// end.
#[derive(Clone)]
pub struct Live {
    /// Replies and requests to the backend.
    pub control: mpsc::Sender<wire::Frame>,
    /// Cancelled when the connection ends; cancelling it ends the
    /// connection.
    pub closed: CancellationToken,
}

/// How work running under the runner reaches the backend and the
/// registration's owner.
#[derive(Clone)]
pub struct ConnectionHandle {
    requests: mpsc::Sender<Request>,
    reach: watch::Receiver<Reach>,
}

/// What work asks of the registration's owner.
pub enum Request {
    /// Where a callback call's events go, until `ended` is cancelled.
    Call {
        id: String,
        events: mpsc::Sender<CallEvent>,
        ended: CancellationToken,
    },
    /// A question to the backend; the asker cancels `abandoned` when it no
    /// longer waits for the answer.
    Ask {
        question: Question,
        abandoned: CancellationToken,
    },
    /// Makes a job's execution context live.
    Context {
        context: Arc<ExecutionContext>,
        leases: PackageLeases,
        reply: oneshot::Sender<io::Result<()>>,
    },
}

impl ConnectionHandle {
    /// A handle whose requests arrive at the returned receiver and that
    /// reaches the backend as `reach` says.
    pub fn new(reach: watch::Receiver<Reach>) -> (Self, mpsc::Receiver<Request>) {
        let (requests, received) = mpsc::channel(REQUESTS);
        (Self { requests, reach }, received)
    }

    /// The connection that serves now, if one does.
    pub fn current(&self) -> Option<Live> {
        match &*self.reach.borrow() {
            Reach::Connected(live) if !live.closed.is_cancelled() => Some(live.clone()),
            _ => None,
        }
    }

    /// The connection that serves, waiting for the next while the runner
    /// is away; an error once the runner ends.
    pub async fn connected(&self) -> Result<Live, Unreachable> {
        let mut reach = self.reach.clone();
        loop {
            match &*reach.borrow_and_update() {
                Reach::Connected(live) if !live.closed.is_cancelled() => return Ok(live.clone()),
                Reach::Unreached => return Err(Unreachable),
                Reach::Connected(_) | Reach::Away => {}
            }
            // A registration that ended reaches nothing more.
            if reach.changed().await.is_err() {
                return Err(Unreachable);
            }
        }
    }

    async fn request(&self, request: Request) -> Result<(), RuntimeError> {
        self.requests
            .send(request)
            .await
            .map_err(|_| RuntimeError::Cancelled)
    }

    /// Sends the call's inbound events to `events` until `ended` is
    /// cancelled, or until the connection that serves now ends; false when
    /// the registration ended.
    pub async fn register_call(
        &self,
        id: String,
        events: mpsc::Sender<CallEvent>,
        ended: CancellationToken,
    ) -> bool {
        self.request(Request::Call { id, events, ended })
            .await
            .is_ok()
    }

    /// Asks the backend where the artifact `sha256` is, on behalf of the live
    /// work `owner` names (`crates/runner-command-packages/src/cache.rs`).
    pub async fn locate(
        &self,
        owner: wire::ArtifactOwner,
        sha256: String,
    ) -> Result<ArtifactLocation, RuntimeError> {
        self.ask(
            |reply| Question::Locate {
                owner,
                sha256,
                reply,
            },
            None,
        )
        .await?
        .map_err(RuntimeError::Location)
    }

    /// Asks the backend a question and waits for its answer while the
    /// connection it was asked on lasts, and for at most `deadline` when it
    /// names one, the name of what was asked and how long it may take; a
    /// question waits for a connection first.
    async fn ask<T>(
        &self,
        question: impl FnOnce(oneshot::Sender<Result<T, String>>) -> Question,
        deadline: Option<(&'static str, Duration)>,
    ) -> Result<Result<T, String>, RuntimeError> {
        let live = self
            .connected()
            .await
            .map_err(|_| RuntimeError::Location(UNREACHABLE.into()))?;
        let (reply, answer) = oneshot::channel();
        let abandoned = CancellationToken::new();
        let _abandon = abandoned.clone().drop_guard();
        self.request(Request::Ask {
            question: question(reply),
            abandoned,
        })
        .await?;
        let answer = async {
            match deadline {
                None => Ok(answer.await),
                Some((what, within)) => tokio::time::timeout(within, answer)
                    .await
                    .map_err(|_| RuntimeError::Deadline(what)),
            }
        };
        tokio::select! {
            _ = live.closed.cancelled() => Err(RuntimeError::Cancelled),
            answer = answer => answer?.map_err(|_| RuntimeError::Cancelled),
        }
    }

    /// Makes `context` live, with the leases that keep its services
    /// resident.
    pub async fn register_context(
        &self,
        context: Arc<ExecutionContext>,
        leases: PackageLeases,
    ) -> io::Result<()> {
        let (reply, answer) = oneshot::channel();
        let ended = || io::Error::other("the runner is stopping");
        self.request(Request::Context {
            context,
            leases,
            reply,
        })
        .await
        .map_err(|_| ended())?;
        answer.await.map_err(|_| ended())?
    }
}

/// The runner ends, and waits for no connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the backend is unreachable")]
pub struct Unreachable;

/// A service's conversation numbers come from the connection it started
/// under (`crates/command-protocol/src/numbers.rs`), and its messages to
/// its plugin go there (§ Messages to the plugin).
impl ServiceBackend for ConnectionHandle {
    fn reserve(
        &self,
        conversation: String,
        sequence: ServiceSequence,
        count: u32,
    ) -> BoxFuture<'_, Result<u64, String>> {
        Box::pin(async move {
            self.ask(
                |reply| Question::Reserve {
                    conversation,
                    sequence,
                    count,
                    reply,
                },
                Some(("conversation numbers", NUMBERS_TIMEOUT)),
            )
            .await
            .map_err(|error| error.to_string())?
        })
    }

    fn tell(
        &self,
        package: String,
        conversation: String,
        message: serde_json::Value,
    ) -> BoxFuture<'_, Result<(), String>> {
        let message = wire::Outbound::PackageMessage {
            package,
            conversation_id: conversation,
            message,
        };
        Box::pin(async move { self.send(&message).await })
    }

    fn lost(&self, package: String) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            // A runner that ends stops its services anyway, and
            // its next start names another instance, which says the same.
            if let Err(error) = self.send(&wire::Outbound::PackageLost { package }).await {
                tracing::info!("a lost service was not reported: {error}");
            }
        })
    }
}

impl ConnectionHandle {
    /// Hands `message` to the connection that serves, waiting for room in
    /// it, and for the next connection while the runner is away or the one
    /// it waited on ended; an error once the runner ends, or for
    /// a message that breaks the wire's rules.
    async fn send(&self, message: &wire::Outbound) -> Result<(), String> {
        let frame = wire::encode(message).map_err(|error| error.to_string())?;
        loop {
            let live = self.connected().await.map_err(|_| UNREACHABLE.to_owned())?;
            if live.control.send(frame.clone()).await.is_ok() {
                return Ok(());
            }
        }
    }
}

/// What work asks the backend, with where the answer goes.
pub enum Question {
    /// Where an artifact is, on behalf of `owner`.
    Locate {
        owner: wire::ArtifactOwner,
        sha256: String,
        reply: oneshot::Sender<Result<ArtifactLocation, String>>,
    },
    /// Numbers of a conversation's sequence for a service.
    Reserve {
        conversation: String,
        sequence: ServiceSequence,
        count: u32,
        reply: oneshot::Sender<Result<u64, String>>,
    },
}

/// Where the answer to a question goes.
enum Reply {
    Location(oneshot::Sender<Result<ArtifactLocation, String>>),
    Numbers(oneshot::Sender<Result<u64, String>>),
}

/// When an entry of the relay no longer needs keeping.
pub enum Ended {
    Call(String),
    Ask(String),
}

/// The owner's routing of callback events and the answers to questions:
/// each entry stays until its inbound message arrives or its asker leaves.
#[derive(Default)]
pub struct Relay {
    calls: HashMap<String, Call>,
    asks: HashMap<String, Reply>,
}

struct Call {
    events: mpsc::Sender<CallEvent>,
    ended: CancellationToken,
}

impl Relay {
    pub fn call(
        &mut self,
        id: String,
        events: mpsc::Sender<CallEvent>,
        ended: CancellationToken,
        watches: &mut JoinSet<Ended>,
    ) {
        let watched = ended.clone();
        let key = id.clone();
        watches.spawn(async move {
            watched.cancelled().await;
            Ended::Call(key)
        });
        self.calls.insert(id, Call { events, ended });
    }

    /// Registers a question and returns the frame that asks it.
    pub fn ask(
        &mut self,
        question: Question,
        abandoned: CancellationToken,
        watches: &mut JoinSet<Ended>,
    ) -> Result<wire::Frame, wire::WireError> {
        let id = uuid::Uuid::new_v4().simple().to_string();
        let (message, reply) = match question {
            Question::Locate {
                owner,
                sha256,
                reply,
            } => (
                wire::Outbound::ArtifactResolve {
                    id: id.clone(),
                    owner,
                    sha256,
                    target: host_target().into(),
                },
                Reply::Location(reply),
            ),
            Question::Reserve {
                conversation,
                sequence,
                count,
                reply,
            } => (
                wire::Outbound::NumbersReserve {
                    id: id.clone(),
                    conversation_id: conversation,
                    sequence,
                    count,
                },
                Reply::Numbers(reply),
            ),
        };
        let frame = wire::encode(&message)?;
        let key = id.clone();
        watches.spawn(async move {
            abandoned.cancelled().await;
            Ended::Ask(key)
        });
        self.asks.insert(id, reply);
        Ok(frame)
    }

    pub fn ended(&mut self, ended: Ended) {
        match ended {
            Ended::Call(id) => {
                self.calls.remove(&id);
            }
            Ended::Ask(id) => {
                self.asks.remove(&id);
            }
        }
    }

    /// Delivers `message` when it answers a call or a question; false for
    /// any other message.
    pub fn route(&mut self, message: &Inbound) -> bool {
        let (id, event) = match message {
            Inbound::RpcPipes {
                call_id,
                stdin,
                stdout,
            } => (
                call_id,
                CallEvent::Pipes {
                    stdin: stdin.clone(),
                    stdout: stdout.clone(),
                },
            ),
            Inbound::RpcOutput { call_id, bytes } => {
                (call_id, CallEvent::Stderr(bytes.0.clone().into()))
            }
            Inbound::RpcStdinPull { call_id } => (call_id, CallEvent::Pull),
            Inbound::RpcExit { call_id, exit_code } => (call_id, CallEvent::Exit(*exit_code)),
            Inbound::ArtifactLocation {
                id,
                location,
                error,
            } => {
                if let Some(Reply::Location(reply)) = self.asks.remove(id) {
                    let answer = match (location, error) {
                        (Some(location), None) => Ok(location.clone()),
                        (None, Some(error)) => Err(error.clone()),
                        _ => Err("invalid artifact location response".into()),
                    };
                    // An asker that left no longer needs the answer.
                    let _left = reply.send(answer);
                }
                return true;
            }
            Inbound::NumbersReserved { id, first, error } => {
                if let Some(Reply::Numbers(reply)) = self.asks.remove(id) {
                    let answer = match (first, error) {
                        (Some(first), None) => Ok(*first),
                        (None, Some(error)) => Err(error.clone()),
                        _ => Err("invalid numbers response".into()),
                    };
                    // An asker that left no longer needs the answer.
                    let _left = reply.send(answer);
                }
                return true;
            }
            _ => return false,
        };
        // A call that cannot keep up with its events ends.
        if let Some(call) = self.calls.get(id)
            && call.events.try_send(event).is_err()
        {
            call.ended.cancel();
        }
        true
    }
}
