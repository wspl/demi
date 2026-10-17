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
//! - [`frames`]: the stream's frames, the pictures' header;
//! - [`writer`]: control before video, video only while the path takes it,
//!   heartbeats;
//! - [`stream`]: one capture per watched source, shared by its viewers, with
//!   key frames on request and the encoding the slowest viewer takes;
//! - [`delivery`]: one viewer's frames in flight, acknowledgements and rate
//!   adaptation;
//! - [`input`]: one viewer's input in order, merged while the source is busy,
//!   and what it holds released when it leaves.

pub mod delivery;
pub mod frames;
pub mod input;
mod rate;
pub mod stream;
pub mod writer;
