//! The browser's REST contract: every request and response body the backend
//! serves, the error body with the one list of its codes, and the identifier
//! and text types those bodies use (`web-api.md`). The backend decodes each
//! request into its type and serializes each response from its type. The
//! data the browser, the agent and the backend share, such as times, comes
//! from `core`.
//!
//! A type the backend receives refuses unknown fields; a type only the
//! browser receives accepts them (`contracts.md` § Generated TypeScript).

pub mod auth;
pub mod error;
pub mod ids;
pub mod settings;
pub mod state;
pub mod text;
