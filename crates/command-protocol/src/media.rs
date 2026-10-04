//! The media a declared command returns (`commands.md` § Return media):
//! images and videos sent where the calling process's stdout goes, their
//! bound, and how their bytes are recognized.

use serde::{Deserialize, Serialize};

/// The largest medium a command returns.
pub const MAX_MEDIUM_BYTES: u64 = 16 * 1024 * 1024;

/// Where a calling process's stdout goes (`runner.md` § Where a command's
/// stdout goes): the job's output, the pipe the runner reads as the job's
/// stdout, or anything else, such as a pipe to another program, a file or a
/// command substitution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StdoutTarget {
    Job,
    Elsewhere,
}

/// The media type `bytes` begin with, from their magic numbers, or `None`
/// for anything but the image and video types a model reads
/// (`models.md` § Accepted attachment types). Fewer than 12 bytes are never
/// recognized. An ISO media file (`ftyp`) is QuickTime for the brand `qt  `,
/// M4V for a brand starting `M4V`, and MP4 otherwise; an EBML header is
/// WebM, the Matroska format a model reads.
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
