//! The media a declared command returns (`commands.md` § Return media):
//! images, videos and PDF documents that `demi file view` hands to its job,
//! their bound, how their bytes are recognized, and the facts a medium
//! record carries of one.

use serde::{Deserialize, Serialize};

/// The largest medium a command returns.
pub const MAX_MEDIUM_BYTES: u64 = 16 * 1024 * 1024;

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

/// What a medium record says of the medium whose bytes follow
/// (`native-runtime.md` § Response records and completion): its media type,
/// a document's name, and the facts its header gives, which the handler
/// read once: an image's or a video's size in pixels, a video's length.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MediumFacts {
    pub media_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}
