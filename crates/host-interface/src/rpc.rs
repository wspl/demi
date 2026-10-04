//! The rpc handler interface (`contracts.md` § The TypeScript boundary): a
//! handler receives its call as data ([`RpcInvocation`]) and acts only
//! through a port whose every operation is one request and one reply, so a
//! process of its own could serve the same calls over a wire.

use std::{collections::BTreeMap, rc::Rc};

use bytes::Bytes;
use demi_command_protocol::CommandContext;
use demi_shared_types::B64Bytes;
use futures_util::future::LocalBoxFuture;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use serde_with::rust::unwrap_or_skip;
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};

use crate::JobCaller;

/// One `rpc` call: the leaf it names, its validated arguments, and the
/// invoking process's surroundings (`commands.md` § Handle an rpc call).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RpcInvocation {
    /// The leaf's path, root first.
    pub path: Vec<String>,
    /// The command line after the root's name, as the process wrote it.
    pub argv: Vec<String>,
    /// The arguments, valid against the leaf's input; a `stdinField` body is
    /// among them.
    pub args: Map<String, Value>,
    /// Whether the caller passed `--json`.
    pub json: bool,
    pub cwd: String,
    pub env: BTreeMap<String, String>,
    /// The invoking job's command context, from the backend's record of it.
    pub context: CommandContext,
    /// The agent node the invoking job runs for, which a job the handler
    /// starts elsewhere carries on; none for a job no agent started.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub caller: Option<JobCaller>,
    /// Whether the calling process has a pipe on its standard input.
    pub stdin: bool,
    /// The pipes relayed for the call's standard input and output, which a
    /// handler that starts a job elsewhere can hand to it.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub pipes: Option<RelayedPipes>,
}

/// The ids of a call's relayed pipes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RelayedPipes {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    pub stdin: Option<String>,
    pub stdout: String,
}

/// The handler of an `rpc` leaf. Its exit code follows the call's standard
/// output: the caller has read everything before the call exits.
pub trait RpcHandler {
    fn call(
        &self,
        invocation: RpcInvocation,
        port: RpcPort,
    ) -> LocalBoxFuture<'_, Result<u8, RpcError>>;
}

/// Why a handler gave up.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RpcError {
    /// The call does not fit the command, such as arguments its input
    /// refuses.
    #[error("{0}")]
    Usage(String),
    /// The handler failed.
    #[error("{0}")]
    Failed(String),
    #[error(transparent)]
    Port(#[from] PortError),
}

/// Why a port operation failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PortError {
    /// The call is over: it was cancelled or stopped, or the calling process
    /// went away, so the port serves nothing more.
    #[error("{0}")]
    Ended(String),
    /// The operation failed in a way its caller could not cause, such as
    /// a database that does not answer.
    #[error("{0}")]
    Failed(String),
    /// The transport answered something other than what was asked.
    #[error("the rpc port answered {answered} to a {asked} request")]
    Unexpected {
        asked: &'static str,
        answered: &'static str,
    },
}

/// One request of a handler through its port.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PortRequest {
    /// Standard output, which reaches the caller through the call's relayed
    /// pipe: the reply waits until the caller has read enough.
    Stdout {
        bytes: B64Bytes,
    },
    Stderr {
        bytes: B64Bytes,
    },
    /// The next chunk of a finite standard input.
    ReadStdin {},
    /// The next interactive write to the calling job, until it ends.
    ReadLiveStdin {},
}

/// The reply to a [`PortRequest`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PortResponse {
    /// The output was written.
    Written {},
    /// A chunk of input; none once the input has ended.
    Input {
        #[serde(deserialize_with = "Option::deserialize")]
        bytes: Option<B64Bytes>,
    },
}

/// What carries a port's requests: the backend's relay of a runner's call,
/// or a test's memory.
pub trait PortTransport {
    fn request(&self, request: PortRequest) -> LocalBoxFuture<'_, Result<PortResponse, PortError>>;
}

/// The handler's side of a call: every operation is one request and one
/// reply, and cancellation says whether and when the call was stopped.
#[derive(Clone)]
pub struct RpcPort {
    transport: Rc<dyn PortTransport>,
    cancel: CancellationToken,
}

impl RpcPort {
    pub fn new(transport: Rc<dyn PortTransport>, cancel: CancellationToken) -> Self {
        Self { transport, cancel }
    }

    /// Writes to standard output, once the caller has room for it.
    pub async fn stdout(&self, bytes: impl Into<Bytes>) -> Result<(), PortError> {
        let request = PortRequest::Stdout {
            bytes: B64Bytes::new(bytes),
        };
        written(self.transport.request(request).await?, "stdout")
    }

    pub async fn stderr(&self, bytes: impl Into<Bytes>) -> Result<(), PortError> {
        let request = PortRequest::Stderr {
            bytes: B64Bytes::new(bytes),
        };
        written(self.transport.request(request).await?, "stderr")
    }

    /// The next chunk of the call's finite standard input; none at its end,
    /// or when the calling process has no pipe there.
    pub async fn read_stdin(&self) -> Result<Option<Bytes>, PortError> {
        input(
            self.transport.request(PortRequest::ReadStdin {}).await?,
            "read_stdin",
        )
    }

    /// The next interactive write to the calling job; none once it ended.
    pub async fn read_live_stdin(&self) -> Result<Option<Bytes>, PortError> {
        input(
            self.transport
                .request(PortRequest::ReadLiveStdin {})
                .await?,
            "read_live_stdin",
        )
    }

    /// Completes once the call is stopped: cancelled by the runner, or ended
    /// by its job, a failed pipe or the backend going away.
    pub fn cancelled(&self) -> WaitForCancellationFuture<'_> {
        self.cancel.cancelled()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
    }

    /// Sends `request` as it is: what a handler elsewhere, such as a plugin,
    /// asks of the call through a port of its own.
    pub async fn forward(&self, request: PortRequest) -> Result<PortResponse, PortError> {
        self.transport.request(request).await
    }

    /// The call's cancellation, which a port that forwards to this one
    /// shares.
    pub fn cancellation(&self) -> CancellationToken {
        self.cancel.clone()
    }
}

fn written(response: PortResponse, asked: &'static str) -> Result<(), PortError> {
    match response {
        PortResponse::Written {} => Ok(()),
        other => Err(unexpected(asked, &other)),
    }
}

fn input(response: PortResponse, asked: &'static str) -> Result<Option<Bytes>, PortError> {
    match response {
        PortResponse::Input { bytes } => Ok(bytes.map(B64Bytes::into_bytes)),
        other => Err(unexpected(asked, &other)),
    }
}

fn unexpected(asked: &'static str, response: &PortResponse) -> PortError {
    let answered = match response {
        PortResponse::Written {} => "written",
        PortResponse::Input { .. } => "input",
    };
    PortError::Unexpected { asked, answered }
}
