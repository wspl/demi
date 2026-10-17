//! What a medium's header says of it (`shell-tool.md` § What `demi file view`
//! shows): an image's size in pixels as a web browser shows it, and a
//! video's size and length where its container's header gives them. The
//! handler reads them once, and they travel with the medium record. Before
//! that, an image is decoded whole, so a picture cut short is refused
//! rather than shown to the model as its header promises.

use std::io::Cursor;

use bytes::Bytes;
use demi_command_protocol::MediumFacts;
use demi_shared_types::whole_image::{self, Undecodable};
use image::{ImageDecoder, ImageFormat, ImageReader, Limits, metadata::Orientation};

/// The most memory reading an image's header may take.
const MAX_HEADER_ALLOC: u64 = 64 * 1024 * 1024;

/// The most memory decoding an image whole may take: a 16 MiB file of
/// about 11,000 × 11,000 pixels.
const MAX_DECODE_ALLOC: u64 = 512 * 1024 * 1024;

/// Decodes `bytes`, an image of `media_type`, whole, so a line printed for
/// it is a picture the model will see (`shell-tool.md` § Media the model
/// views). A type the decoder does not read is taken as it is.
pub(crate) fn decode_whole(bytes: &[u8], media_type: &str) -> Result<(), Undecodable> {
    let Some(format) = ImageFormat::from_mime_type(media_type) else {
        return Ok(());
    };
    match whole_image::decode(bytes, format, MAX_DECODE_ALLOC) {
        Ok(_) | Err(Undecodable::Unknown) => Ok(()),
        Err(undecodable) => Err(undecodable),
    }
}

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
