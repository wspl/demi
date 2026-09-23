//! The building blocks every HTTP provider decodes its vendor's answers with
//! (`providers.md` § Reading vendor input): server-sent event framing, the
//! two-step decode of payloads tagged by `type`, and fields Demi only
//! reports.

mod sse;
mod tagged;

pub use sse::{SseError, sse_data};
pub use tagged::{ReportedString, Tagged, TaggedWire, WireError, decode_tagged};

/// What [`tagged_wire!`](crate::tagged_wire) expands to refers to, so that a
/// crate using the macro needs no dependency of its own for it.
#[doc(hidden)]
pub mod __private {
    pub use serde;
}
