//! One backend connection (`runner.md` § Connection and identity). Its owner
//! routes each message to the work it belongs to and owns everything that
//! lasts as long as the connection: the volumes, the Host requests, the
//! service streams and the raw processes. The owner never waits for that
//! work, so the inbound queue drains; closing the connection ends all of it.
//! What outlives a connection, the jobs with their execution contexts, the
//! installed manifest and their commands' calls, the registration keeps
//! ([`Kept`]), and each connection serves it while it lasts (`runner.md`
//! § Command lifetime).
//!
//! Work that runs apart from the owner reaches it through a
//! `demi_runner_jobs::connection::ConnectionHandle`, whose requests the
//! connection that serves, or the registration between connections, serve.

mod kept;
mod owner;
mod transport;

pub use kept::Kept;
pub use owner::{End, Registered, serve};
pub use transport::{Connected, Transport};
