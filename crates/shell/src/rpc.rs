//! The rpc handler interface (`contracts.md` § The TypeScript boundary): a
//! handler receives its call as data ([`RpcInvocation`]) and acts only
//! through a port whose every operation is one request and one reply, so a
//! process of its own could serve the same calls over a wire.

use std::{collections::BTreeMap, rc::Rc};

use bytes::Bytes;
use demi_command_service::protocol::CommandContext;
use demi_core::B64Bytes;
use futures_util::future::LocalBoxFuture;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
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
    /// Whose command storage the invoking job reaches, which a job the
    /// handler starts elsewhere carries on; none for a job no agent started.
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
    /// Command storage refused the operation.
    #[error("{0}")]
    Storage(String),
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
    Storage {
        op: StorageOp,
    },
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
    Storage {
        reply: StorageReply,
    },
}

/// An operation on the invoking agent node's command storage
/// (`command-state-history.md` § Mutation API and concurrency).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "op",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum StorageOp {
    /// The key's value in the current version, and the node's revision.
    Read { key: String },
    /// The keys that start with `prefix`, sorted.
    List { prefix: String },
    /// Sets the key, or removes it when `value` is none, if the node's
    /// revision is still `expected`; without `expected`, whatever it is.
    WriteIf {
        key: String,
        #[serde(deserialize_with = "Option::deserialize")]
        value: Option<Value>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            with = "unwrap_or_skip"
        )]
        expected: Option<Revision>,
    },
}

/// The answer to a [`StorageOp`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "outcome",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum StorageReply {
    Value {
        #[serde(deserialize_with = "Option::deserialize")]
        value: Option<Value>,
        revision: Revision,
    },
    Keys {
        keys: Vec<String>,
    },
    /// The write is the node's current version, `revision`.
    Committed {
        revision: Revision,
    },
    /// Another write came first; the node is at `revision`.
    Conflict {
        revision: Revision,
    },
}

/// The version a node's command storage is at. Every committed write of any
/// of its keys advances it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Revision(pub u64);

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

    pub async fn storage(&self, op: StorageOp) -> Result<StorageReply, PortError> {
        match self.transport.request(PortRequest::Storage { op }).await? {
            PortResponse::Storage { reply } => Ok(reply),
            other => Err(unexpected("storage", &other)),
        }
    }

    /// Replaces `key` by what `change` makes of its current value: reads,
    /// computes, and writes if nothing was written since, starting again on
    /// a conflict. `change` does no other IO, so running it again is
    /// harmless. A stored value `T` cannot decode is an error, never
    /// replaced.
    pub async fn update<T, F>(&self, key: &str, mut change: F) -> Result<T, RpcError>
    where
        T: Serialize + DeserializeOwned,
        F: FnMut(Option<T>) -> Result<T, RpcError>,
    {
        loop {
            let (stored, revision) = match self.storage(StorageOp::Read { key: key.into() }).await?
            {
                StorageReply::Value { value, revision } => (value, revision),
                other => return Err(unexpected_reply("read", &other).into()),
            };
            let current = stored
                .map(serde_json::from_value::<T>)
                .transpose()
                .map_err(|error| {
                    RpcError::Failed(format!("stored {key} is unreadable: {error}"))
                })?;
            let next = change(current)?;
            let value = serde_json::to_value(&next)
                .map_err(|error| RpcError::Failed(format!("{key} cannot be stored: {error}")))?;
            let write = StorageOp::WriteIf {
                key: key.into(),
                value: Some(value),
                expected: Some(revision),
            };
            match self.storage(write).await? {
                StorageReply::Committed { .. } => return Ok(next),
                StorageReply::Conflict { .. } => continue,
                other => return Err(unexpected_reply("write_if", &other).into()),
            }
        }
    }

    /// Completes once the call is stopped: cancelled by the runner, or ended
    /// by its job, a failed pipe or the backend going away.
    pub fn cancelled(&self) -> WaitForCancellationFuture<'_> {
        self.cancel.cancelled()
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancel.is_cancelled()
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
        PortResponse::Storage { .. } => "storage",
    };
    PortError::Unexpected { asked, answered }
}

fn unexpected_reply(asked: &'static str, reply: &StorageReply) -> PortError {
    let answered = match reply {
        StorageReply::Value { .. } => "value",
        StorageReply::Keys { .. } => "keys",
        StorageReply::Committed { .. } => "committed",
        StorageReply::Conflict { .. } => "conflict",
    };
    PortError::Unexpected { asked, answered }
}
