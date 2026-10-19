//! The Host's side of the live view where it is not about its source
//! (`live-view.md` § The stream, `mobile.md` § The live view): a source of
//! pictures and input, such as a browser tab or a simulator, implements
//! [`stream::Source`] and [`input::Deliver`], and its viewer serves its
//! user stream with these pieces:
//!
//! ```text
//!  page ──frames──▶ frames::Reader ──▶ the source's viewer ──▶ input::Input ──▶ Deliver
//!  page ◀──────── writer::Writer ◀── delivery::Delivery ◀── stream::StreamHandle ◀── Source
//! ```
//!
//! - [`frames`]: the stream's frames, the pictures' chunks;
//! - [`writer`]: control before video, heartbeats;
//! - [`stream`]: one capture per watched source, shared by its viewers, with
//!   key frames on request and the encoding the slowest viewer takes;
//! - [`delivery`]: one viewer's chunks paced over each frame's interval,
//!   the window of what is unacknowledged, and the target bit rate from the
//!   page's acknowledgements, with the delay-based estimate of Google
//!   Congestion Control ported from str0m;
//! - [`input`]: one viewer's input in order, merged while the source is busy,
//!   and what it holds released when it leaves.

mod bwe;
pub mod delivery;
pub mod frames;
pub mod input;
mod rate;
pub mod stream;
pub mod writer;
