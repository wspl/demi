//! The backend's end of a runner (`crates-and-packages.md` § host-remote):
//!
//! - the connection engine ([`Link`], served by its [`LinkDriver`]): replies
//!   routed by id, liveness, and the plumbing of the rpc calls a job's
//!   declared commands make;
//! - [`RemoteHost`], a Host over its device's connection, with job,
//!   working-tree, network, log and service facets, whose file contents
//!   travel through pipes;
//! - pipe records ([`Pipes`]) and their `Send` ends;
//! - [`RemoteShellEnvironment`], the shell behind the `shell_*` tools over
//!   real runner jobs, and its factory;
//! - manifests built from a command set ([`CommandCatalog`]).
//!
//! It owns no socket and no HTTP route: the backend's connection tasks and
//! pipe routes feed it. Everything but the pipe ends runs inside the user's
//! shard.

mod link;
mod manifest;
mod pipes;
mod relay;
mod remote_host;
mod shell_environment;
#[cfg(feature = "testing")]
pub mod testing;

pub use link::{
    JobEnd, JobOrigin, Link, LinkDriver, LinkEnd, LinkOptions, LinkPolicy, OUTBOUND_FRAMES,
    PING_INTERVAL,
};
pub use manifest::{ArtifactResolver, CommandCatalog, CommandSelection};
pub use pipes::{
    ARRIVAL, DeviceSink, DeviceSource, Pipe, PipeError, PipeFailure, PipeReader, PipeRefusal,
    PipeWriter, Pipes,
};
pub use remote_host::{
    Admission, DeviceLink, JobStart, LogPage, RemoteHost, RemoteJob, ServiceCallError, ServiceEnd,
    ServiceRequest, ServiceStream, host_identity,
};
pub use shell_environment::{
    ContextSource, EnvironmentOptions, HostAccess, RemoteShellEnvironment,
    RemoteShellEnvironmentFactory, RetainEdits, edited_file,
};
