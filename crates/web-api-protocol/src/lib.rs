//! The web app's REST contract: every request and response body the backend
//! serves, the error body with the one list of its codes, and the identifier
//! and text types those bodies use (`web-api.md`). The backend decodes each
//! request into its type and serializes each response from its type. The
//! data the web app, the agent and the backend share, such as times, comes
//! from `core`.
//!
//! A type the backend receives refuses unknown fields; a type only the
//! web app receives accepts them (`contracts.md` § Generated TypeScript).

pub mod attachments;
pub mod auth;
pub mod cloud;
pub mod conversations;
pub mod devices;
pub mod drafts;
pub mod error;
pub mod exposes;
pub mod files;
pub mod hosts;
pub mod ids;
pub mod panel;
pub mod plugins;
pub mod providers;
pub mod query;
pub mod settings;
pub mod sidebar;
pub mod state;
pub mod text;
pub mod usage;
pub mod users;
pub mod workspaces;

/// The largest message a page sends on any of its WebSockets: the
/// conversation socket, the synchronization channel and a user stream
/// (`web-api.md` § Request bodies). A frame refers to an upload and never
/// carries its bytes, and a user stream frames its own messages, so no page
/// needs a larger one.
pub const MAX_PAGE_MESSAGE_BYTES: usize = 1024 * 1024;
