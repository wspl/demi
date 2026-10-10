//! A medium's size in pixels, read from its header alone (`runtime.md`
//! § Media), which its reference and an attachment's record carry so a page
//! knows a thumbnail's box before the bytes arrive: an image's as it shows,
//! and a video's from its container's header, MP4 and QuickTime or WebM.

use bytes::Bytes;
use demi_command_protocol::medium_facts;
use demi_shared_types::PixelSize;

/// The size of `bytes`, a medium of `media_type`: an image's as a web
/// browser shows it, and a video's where its container's header gives it.
/// None for any other medium, and for one whose header does not read.
pub fn pixel_size(bytes: Bytes, media_type: &str) -> Option<PixelSize> {
    medium_facts(&bytes, media_type)
        .size
        .map(|(width, height)| PixelSize { width, height })
}
