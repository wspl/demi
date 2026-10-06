//! A medium's size in pixels, read from its header alone (`runtime.md`
//! § Media), which its reference and an attachment's record carry so a page
//! knows a thumbnail's box before the bytes arrive: an image's as it shows,
//! and a video's from its container's header, MP4 and QuickTime or WebM.

use bytes::Bytes;
use demi_shared_types::PixelSize;

use crate::images;

/// The size of `bytes`, a medium of `media_type`: an image's as a web
/// browser shows it, and a video's where its container's header gives it.
/// None for any other medium, and for one whose header does not read.
pub fn pixel_size(bytes: Bytes, media_type: &str) -> Option<PixelSize> {
    match media_type.split_once('/')?.0 {
        "image" => images::shown_size(&bytes, media_type),
        "video" => video_size(bytes),
        _ => None,
    }
}

/// The size a video's container header gives its video track. The parser
/// walks the container's boxes or elements to the track header and never
/// reads the frames; a header after the frames, as in an MP4 whose `moov`
/// box comes last, is found only when `bytes` reaches it.
fn video_size(bytes: Bytes) -> Option<PixelSize> {
    let source = nom_exif::MediaSource::from_memory(bytes).ok()?;
    let track = nom_exif::MediaParser::new().parse_track(source).ok()?;
    let side = |tag| {
        track
            .get(tag)
            .and_then(nom_exif::EntryValue::as_u32)
            .filter(|side| *side > 0)
    };
    Some(PixelSize {
        width: side(nom_exif::TrackInfoTag::Width)?,
        height: side(nom_exif::TrackInfoTag::Height)?,
    })
}
