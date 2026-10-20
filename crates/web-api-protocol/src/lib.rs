//! The web app's REST contract: every request and response body the backend
//! serves, the error body with the one list of its codes, and the identifier
//! and text types those bodies use.
//!
//! The backend decodes each request into its type and serializes each response
//! from its type. The data the web app, the agent and the backend share, such
//! as times, comes from `core`.
//!
//! - A type the backend receives refuses unknown fields. A type only the web
//!   app receives accepts them (`crates/xtask/src/contracts/mod.rs`).
//! - A response carries the fields its type declares and nothing more: what the
//!   backend keeps for itself in a stored record, such as a conversation's
//!   owner or its message counts, never reaches the web app.
//! - The crate reuses the runner's working-tree change types and platform, and
//!   the command locale, rather than declaring them again.
//! - A Host log line is its own type: the runner wire carries its time as
//!   integer milliseconds and the web app receives an RFC 3339 time.
//!
//! The crate holds no route handling and no domain logic.

pub mod attachments;
pub mod auth;
pub mod automations;
pub mod cloud;
pub mod conversations;
pub mod credentials;
pub mod devices;
pub mod diagnostics;
pub mod drafts;
pub mod error;
pub mod exposed_ports;
pub mod files;
pub mod hosts;
pub mod ids;
pub mod instructions;
pub mod panel;
pub mod passkeys;
pub mod permissions;
pub mod plugins;
pub mod port_forwards;
pub mod providers;
pub mod push;
pub mod query;
pub mod saved;
pub mod search;
pub mod sessions;
pub mod settings;
pub mod shares;
pub mod side_questions;
pub mod sidebar;
pub mod state;
pub mod subagents;
pub mod text;
pub mod usage;
pub mod users;
pub mod workflows;
pub mod workspaces;

/// The largest message a page sends on any of its WebSockets: the
/// conversation socket, the synchronization channel and a user stream. A frame
/// refers to an upload and never carries its bytes, and a user stream frames
/// its own messages, so no page needs a larger one.
pub const MAX_PAGE_MESSAGE_BYTES: usize = 1024 * 1024;
