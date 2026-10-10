//! What a medium's header says of it (`runtime.md` § What `demi file view`
//! shows): an image's size in pixels as a web browser shows it, and a
//! video's size and length where its container's header gives them. The
//! handler reads them once, and they travel with the medium record.

use std::io::Cursor;

use bytes::Bytes;
use demi_command_protocol::MediumFacts;
use image::{ImageDecoder, ImageFormat, ImageReader, Limits, metadata::Orientation};

/// The most memory reading an image's header may take.
const MAX_HEADER_ALLOC: u64 = 64 * 1024 * 1024;

/// The facts of `bytes`, a medium of `media_type` named `name` when it is a
/// document; only the header is read, and one that does not read says
/// nothing of the size.
pub(crate) fn facts(bytes: &Bytes, media_type: &str, name: Option<String>) -> MediumFacts {
    let mut facts = MediumFacts {
        media_type: media_type.to_owned(),
        ..MediumFacts::default()
    };
    match media_type.split_once('/').map(|(kind, _)| kind) {
        Some("image") => {
            (facts.width, facts.height) = image_size(bytes, media_type).unzip();
        }
        Some("video") => video_header(bytes.clone(), &mut facts),
        _ => facts.name = name,
    }
    facts
}

/// The size an image shows at: its pixels turned upright as its
/// orientation says.
fn image_size(bytes: &[u8], media_type: &str) -> Option<(u32, u32)> {
    let format = ImageFormat::from_mime_type(media_type)?;
    let mut reader = ImageReader::with_format(Cursor::new(bytes), format);
    let mut limits = Limits::default();
    limits.max_alloc = Some(MAX_HEADER_ALLOC);
    reader.limits(limits);
    let mut decoder = reader.into_decoder().ok()?;
    let (width, height) = decoder.dimensions();
    let turned = matches!(
        decoder.orientation().ok()?,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    );
    Some(if turned {
        (height, width)
    } else {
        (width, height)
    })
}

/// The size and length a video's container header gives its video track.
/// The parser walks the container's boxes or elements to the track header
/// and never reads the frames.
fn video_header(bytes: Bytes, facts: &mut MediumFacts) {
    let Some(track) = nom_exif::MediaSource::from_memory(bytes)
        .ok()
        .and_then(|source| nom_exif::MediaParser::new().parse_track(source).ok())
    else {
        return;
    };
    let side = |tag| {
        track
            .get(tag)
            .and_then(nom_exif::EntryValue::as_u32)
            .filter(|side| *side > 0)
    };
    (facts.width, facts.height) = side(nom_exif::TrackInfoTag::Width)
        .zip(side(nom_exif::TrackInfoTag::Height))
        .unzip();
    facts.duration_ms = track
        .get(nom_exif::TrackInfoTag::DurationMs)
        .and_then(nom_exif::EntryValue::as_u64)
        .filter(|duration| *duration > 0);
}
