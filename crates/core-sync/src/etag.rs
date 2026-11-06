//! The entity tag of a value a page reads or is pushed.

use serde::Serialize;
use sha2::{Digest, Sha256};

/// The entity tag of `body`, the value a read answers with: the quoted hash
/// of its JSON, which is what the `ETag` of the read carries
/// (`crates/backend-http/src/revalidate.rs`) and what a push of the value
/// names.
pub fn etag_of<T: Serialize>(body: &T) -> String {
    // The messages' types serialize as JSON, which serde_json never refuses.
    etag_of_json(&serde_json::to_vec(body).expect("a body serializes to JSON"))
}

/// The entity tag of the JSON `json`.
pub fn etag_of_json(json: &[u8]) -> String {
    format!("\"{:x}\"", Sha256::digest(json))
}
