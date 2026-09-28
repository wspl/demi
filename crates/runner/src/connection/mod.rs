//! One backend connection (`runner.md` § Connection and identity). Its owner
//! routes each message to the work it belongs to and owns everything that
//! lasts as long as the connection: the job table, the execution contexts
//! and installed manifest, the callbacks and artifact locations in flight,
//! the volumes and the Host requests. The owner never waits for that work,
//! so the inbound queue drains; closing the connection ends all of it.
//!
//! Work that runs apart from the owner reaches it through a
//! [`ConnectionHandle`]: a callback call registers where its events go, an
//! artifact download asks where its artifact is, a service asks for its
//! conversation numbers, and a job makes its execution context live.

mod owner;
pub mod transport;

pub use demi_runner_protocol::wire;
pub use owner::{End, Registered, serve};
pub use transport::{Transport, socket_url};

use std::{collections::HashMap, io, sync::Arc, time::Duration};

use demi_command_service::protocol::{ArtifactLocation, ServiceSequence};
use futures_util::future::BoxFuture;
use tokio::{
    sync::{mpsc, oneshot},
    task::JoinSet,
};
use tokio_util::sync::CancellationToken;

use crate::{
    commands::{contexts::ExecutionContext, rpc::CallEvent},
    services::{NumberSource, RuntimeError, ServiceLease},
};
use wire::Inbound;

/// Requests waiting for a connection's owner; a sender waits for room.
const REQUESTS: usize = 64;
/// How long the backend has to answer a question: where an artifact is, or
/// which numbers a service may use.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(15);

/// How work running under a connection reaches the backend and the
/// connection's owner.
#[derive(Clone)]
pub struct ConnectionHandle {
    /// Replies and requests to the backend.
    pub control: mpsc::Sender<wire::Frame>,
    requests: mpsc::Sender<Request>,
    closed: CancellationToken,
}

/// What work asks of a connection's owner.
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
        leases: Vec<ServiceLease>,
        reply: oneshot::Sender<io::Result<()>>,
    },
}

impl ConnectionHandle {
    /// A handle whose requests arrive at the returned receiver; `closed`
    /// ends with the connection.
    pub fn new(
        control: mpsc::Sender<wire::Frame>,
        closed: CancellationToken,
    ) -> (Self, mpsc::Receiver<Request>) {
        let (requests, received) = mpsc::channel(REQUESTS);
        (
            Self {
                control,
                requests,
                closed,
            },
            received,
        )
    }

    /// Cancelled when the connection ends.
    pub fn closed(&self) -> &CancellationToken {
        &self.closed
    }

    async fn request(&self, request: Request) -> Result<(), RuntimeError> {
        tokio::select! {
            _ = self.closed.cancelled() => Err(RuntimeError::Cancelled),
            sent = self.requests.send(request) => sent.map_err(|_| RuntimeError::Cancelled),
        }
    }

    /// Sends the call's inbound events to `events` until `ended` is
    /// cancelled; false when the connection has ended.
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
    /// work `owner` names (`native-runtime.md` § Install the selected
    /// executable).
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
            "artifact location",
        )
        .await?
        .map_err(RuntimeError::Location)
    }

    /// Asks the backend a question and waits for its answer, for at most
    /// `ANSWER_TIMEOUT` and while the connection lasts.
    async fn ask<T>(
        &self,
        question: impl FnOnce(oneshot::Sender<Result<T, String>>) -> Question,
        what: &'static str,
    ) -> Result<Result<T, String>, RuntimeError> {
        let (reply, answer) = oneshot::channel();
        let abandoned = CancellationToken::new();
        let _abandon = abandoned.clone().drop_guard();
        self.request(Request::Ask {
            question: question(reply),
            abandoned,
        })
        .await?;
        tokio::select! {
            _ = self.closed.cancelled() => Err(RuntimeError::Cancelled),
            answer = tokio::time::timeout(ANSWER_TIMEOUT, answer) => answer
                .map_err(|_| RuntimeError::Deadline(what))?
                .map_err(|_| RuntimeError::Cancelled),
        }
    }

    /// Makes `context` live on the connection, with the leases that keep its
    /// services resident.
    pub async fn register_context(
        &self,
        context: Arc<ExecutionContext>,
        leases: Vec<ServiceLease>,
    ) -> io::Result<()> {
        let (reply, answer) = oneshot::channel();
        let closed = || io::Error::other("host connection closed");
        self.request(Request::Context {
            context,
            leases,
            reply,
        })
        .await
        .map_err(|_| closed())?;
        tokio::select! {
            _ = self.closed.cancelled() => Err(closed()),
            answer = answer => answer.map_err(|_| closed())?,
        }
    }
}

/// A service's conversation numbers come from the connection it started
/// under (`native-runtime.md` § Conversation numbers).
impl NumberSource for ConnectionHandle {
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
                "conversation numbers",
            )
            .await
            .map_err(|error| error.to_string())?
        })
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
                    target: crate::services::target().into(),
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
