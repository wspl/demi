//! One backend connection (the wire is `demi_runner_protocol::wire`).
//!
//! Its owner routes each message to the work it belongs to and owns what lasts
//! as long as the connection: the volumes, the Host requests, the resident
//! streams and the raw processes. The owner never waits for that work, so it
//! reads each message as it arrives; closing the connection ends all of it.
//!
//! What outlives a connection, the registration keeps ([`Kept`]), and each
//! connection serves it while it lasts: the jobs with their execution contexts,
//! the installed manifest and their commands' calls.
//!
//! The link's numbers outlive each connection ([`LinkState`]): a connection
//! that replaces another carries on from the message the backend got to, and
//! the calls the jobs' commands make go on across the drop.
//!
//! The hello names the Host: system and release, architecture, kernel (read
//! once at start without a shell), `SHELL`, installation directory, and why
//! autostart is unavailable. The backend keeps it on the device record and the
//! agent reads it in its context block.
//!
//! Work that runs apart from the owner reaches it through a
//! `demi_runner_jobs::connection::ConnectionHandle`, whose requests the
//! connection that serves, or the registration between connections, serve.

mod kept;
mod link_state;
mod owner;
mod transport;

pub use kept::Kept;
pub use link_state::LinkState;
pub use owner::{End, Registered, serve};
pub use transport::{Connected, Transport};
