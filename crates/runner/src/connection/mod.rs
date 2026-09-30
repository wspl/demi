//! One backend connection (`runner.md` § Connection and identity). Its owner
//! routes each message to the work it belongs to and owns everything that
//! lasts as long as the connection: the job table, the execution contexts
//! and installed manifest, the callbacks and artifact locations in flight,
//! the volumes and the Host requests. The owner never waits for that work,
//! so the inbound queue drains; closing the connection ends all of it.
//!
//! Work that runs apart from the owner reaches it through a
//! `demi_runner_jobs::connection::ConnectionHandle`, whose requests the owner
//! serves.

mod owner;
mod transport;

pub use owner::{End, Registered, serve};
pub use transport::Transport;
