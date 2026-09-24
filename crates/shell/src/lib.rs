//! The contracts between the agent and what executes its work
//! (`crates-and-packages.md` § shell), and nothing that implements them:
//!
//! - the Host contract: an execution target's filesystem and processes
//!   ([`Host`], [`HostFs`], [`HostProcess`]) and its value identity
//!   ([`HostKey`]);
//! - the command system: a [`CommandSet`] pairs declaration trees with the
//!   handler of each `rpc` leaf, built with [`GroupBuilder`] and
//!   [`LeafBuilder`]; a handler receives its call as data
//!   ([`RpcInvocation`]) and acts through an [`RpcPort`] of messages;
//! - the shell-environment contract behind the `shell_*` tools
//!   ([`ShellEnvironment`], [`ExecRequest`], [`CommandStatus`]) and the
//!   record every environment keeps its commands in ([`CommandRecord`]),
//!   which keeps one place in each command's output for the model and one for
//!   the page ([`Reader`]).
//!
//! Everything here runs inside a user's shard, so nothing requires `Send`.

mod builders;
mod commands;
mod environment;
mod host;
mod record;
mod reserved;
mod rpc;
#[cfg(feature = "testing")]
pub mod testing;

pub use builders::{Call, GroupBuilder, LeafBuilder, TypedRpc};
pub use commands::{CommandSet, Declared, RegisterError};
pub use environment::{
    BinaryOutput, CommandState, CommandStatus, DEFAULT_BINARY_LIMIT_BYTES, DEFAULT_OBSERVATION,
    DEFAULT_OUTPUT_LIMIT_BYTES, EditedFiles, ExecRequest, JobCaller, MAX_OBSERVATION,
    ObservationWindow, ShellEnvironment, ShellError, ShellTarget,
};
pub use host::{
    ByteRange, ByteStream, CpOptions, DirEntry, FileContents, FileKind, FileStat, Host, HostError,
    HostErrorKind, HostFs, HostIdentity, HostKey, HostProcess, MkdirOptions, Process,
    ProcessControl, ProcessEnd, ProcessOutput, RmOptions, Signal, SpawnEnv, SpawnError,
    SpawnErrorKind, SpawnRequest, WriteOptions,
};
pub use record::{CommandRecord, Ending, Reader, TAIL_CHARS, final_stdout_boundary};
pub use reserved::{RESERVED_NAMES, is_reserved};
pub use rpc::{
    PortError, PortRequest, PortResponse, PortTransport, RelayedPipes, Revision, RpcError,
    RpcHandler, RpcInvocation, RpcPort, StorageOp, StorageReply,
};
