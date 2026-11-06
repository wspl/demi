//! What the server does for runners beside a core's connections to them
//! (`core-hosts`):
//!
//! - Pairing, with its pending claims and codes.
//! - The installer scripts.
//! - Native package publication, with the artifacts sourced on demand and the
//!   older releases' packages removed.
//! - The next runner release the paired devices download ahead.
//! - The backend's public address that runners reach.
//!
//! Native publication uploads to the object store, which is why the crate
//! depends on `core-blobs`.

pub mod claims;
pub mod codes;
pub mod install;
pub mod native;
pub mod prefetch;
pub mod public_url;
pub mod publication;
pub mod retention;
pub mod sourcing;
