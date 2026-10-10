//! The media a declared command returns (`commands.md` § Return media):
//! images, videos and PDF documents that `demi file view` hands to its job,
//! their bound, how their bytes are recognized, and what their headers say
//! of their size and length.

use std::io::Cursor;

use bytes::Bytes;
use image::{ImageDecoder, ImageFormat, ImageReader, Limits, metadata::Orientation};

/// The largest medium a command returns.
pub const MAX_MEDIUM_BYTES: u64 = 16 * 1024 * 1024;

/// The most memory reading an image's header may take.
const MAX_HEADER_ALLOC: u64 = 64 * 1024 * 1024;

/// The media type `bytes` begin with, from their magic numbers, or `None`
/// for anything but the image, video and document types a model reads
/// (`models.md` § Accepted attachment types). Fewer than 12 bytes are never
/// recognized. An ISO media file (`ftyp`) is QuickTime for the brand `qt  `,
/// M4V for a brand starting `M4V`, and MP4 otherwise; an EBML header is
/// WebM, the Matroska format a model reads; `%PDF-` begins a PDF.
pub fn sniff_media_type(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() < 12 {
        return None;
    }
    let media_type = if bytes.starts_with(b"\x89PNG") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        "image/gif"
    } else if bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else if bytes.starts_with(b"\x1a\x45\xdf\xa3") {
        "video/webm"
    } else if bytes.starts_with(b"%PDF-") {
        "application/pdf"
    } else if &bytes[4..8] == b"ftyp" {
        let brand = &bytes[8..12];
        if brand == b"qt  " {
            "video/quicktime"
        } else if brand.starts_with(b"M4V") {
            "video/x-m4v"
        } else {
            "video/mp4"
        }
    } else {
        return None;
    };
    Some(media_type)
}

/// What a medium's header says of it: an image's size in pixels as a web
/// browser shows it, and a video's size and length where its container's
/// header gives them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct MediumFacts {
    /// Width and height in pixels.
    pub size: Option<(u32, u32)>,
    pub duration_ms: Option<u64>,
}

/// The header of `bytes`, a medium of `media_type`: only the header is
/// read, and one that does not read says nothing.
pub fn medium_facts(bytes: &Bytes, media_type: &str) -> MediumFacts {
    match media_type.split_once('/').map(|(kind, _)| kind) {
        Some("image") => MediumFacts {
            size: image_size(bytes, media_type),
            duration_ms: None,
        },
        Some("video") => video_header(bytes.clone()),
        _ => MediumFacts::default(),
    }
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
/// and never reads the frames; a header after the frames, as in an MP4
/// whose `moov` box comes last, is found only when `bytes` reaches it.
fn video_header(bytes: Bytes) -> MediumFacts {
    let Some(track) = nom_exif::MediaSource::from_memory(bytes)
        .ok()
        .and_then(|source| nom_exif::MediaParser::new().parse_track(source).ok())
    else {
        return MediumFacts::default();
    };
    let side = |tag| {
        track
            .get(tag)
            .and_then(nom_exif::EntryValue::as_u32)
            .filter(|side| *side > 0)
    };
    let size = side(nom_exif::TrackInfoTag::Width).zip(side(nom_exif::TrackInfoTag::Height));
    let duration_ms = track
        .get(nom_exif::TrackInfoTag::DurationMs)
        .and_then(nom_exif::EntryValue::as_u64)
        .filter(|duration| *duration > 0);
    MediumFacts { size, duration_ms }
}
