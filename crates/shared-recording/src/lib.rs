//! A recording of a screen that keeps every frame it drew
//! (`docs/decisions/0036-a-recording-keeps-every-drawn-frame.md`).
//!
//! A source, a browser tab or a simulator, codes each frame its screen drew
//! that differs from the one before, exactly, as H.264 at the screen's own
//! size, and hands the coded picture to a [`Sink`] with its time and where it
//! differs. A recording keeps those pictures as its video and says in an index
//! where each one changed; a still screen costs nothing, and a one-frame
//! change is kept as surely as a long one. What a source could not take it
//! counts, rather than claiming none.
//!
//! ```text
//! Library ---begin---> Live (a thread: write video + index)   Stored
//!   one directory         ^ Sink::push_picture, Sink::end         |
//!   for a conversation    | source                                +-- frames
//!                                                                 +-- export
//!                                                                 +-- write_mp4
//! ```
//!
//! # Boundaries
//!
//! - The crate knows no source, no command and no conversation: a program
//!   gives it a directory and a codec, and its sources' coded pictures. It
//!   encodes nothing; [`changed`] is the test a source makes before it does.
//! - Its files are the only state a recording has, so a program that ends
//!   leaves recordings that can be read; the recording it was writing is
//!   reported interrupted.
//! - An export decodes the video with OpenH264 (`demi-shared-openh264`),
//!   the same everywhere, which is why an exported frame is the video's
//!   decoded picture, not the screen's exact pixels.

mod changed;
mod export;
mod font;
mod held;
mod library;
mod mp4;
mod operate;
mod recorder;
mod stored;

pub use export::{Crop, Export, Exported};
pub use changed::changed;
pub use held::{Held, Settled};
pub use library::{Library, Listing, Stored};
pub use operate::{Answer, Reading, started};
pub use recorder::{Handover, Live, Picture, Sink};

use std::path::{Path, PathBuf};

use demi_shared_recording_protocol::RecordingId;

/// How many pictures a source codes between key frames, every source the
/// same: a recording is decoded from the key frame before the frame wanted,
/// so the end of a long one costs at most this many pictures, not all of it.
/// A source also codes a key frame where its picture changes size.
pub const KEY_INTERVAL: u32 = 120;

/// Why a recording could not be made or read.
#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("there is no recording {0}")]
    NoRecording(RecordingId),
    /// An older recording than those kept, which the five newest pushed out.
    #[error("recording {0} was removed: a conversation keeps only its newest five recordings")]
    Removed(RecordingId),
    #[error("five recordings are running, so no other can start")]
    TooManyRunning,
    /// The encoder cannot be had on this Host.
    #[error("{0}")]
    Unavailable(String),
    #[error("{0}")]
    BadRequest(String),
    /// An image of the export would replace a file that is there.
    #[error("{} exists; pass --overwrite to replace it", .0.display())]
    Exists(PathBuf),
    #[error("{0}")]
    Io(String),
    #[error("a recording's files are corrupt: {0}")]
    Corrupt(String),
    #[error("{0}")]
    Failed(String),
}

impl Error {
    pub(crate) fn io(path: &Path, error: &std::io::Error) -> Self {
        Self::Io(format!("{}: {error}", path.display()))
    }
}
