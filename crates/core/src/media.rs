//! The binary types a model can receive natively, and how to recognize them
//! (`runtime.md` § Results and previews). The set is closed: bytes are known
//! by their magic numbers or not at all, never by guessing from a name.

use crate::{FileExtension, Model, file_extension_support};

/// Whether a model reads a medium as an image or as a video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ModelMediaKind {
    Image,
    Video,
}

/// A media type a model can receive, with the extension a model's catalog
/// accepts it by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModelMediaType {
    pub media_type: &'static str,
    pub kind: ModelMediaKind,
    pub extension: FileExtension,
}

/// Every media type a model can receive natively.
pub const MODEL_MEDIA_TYPES: [ModelMediaType; 8] = [
    media("image/png", ModelMediaKind::Image, FileExtension::Png),
    media("image/jpeg", ModelMediaKind::Image, FileExtension::Jpeg),
    media("image/gif", ModelMediaKind::Image, FileExtension::Gif),
    media("image/webp", ModelMediaKind::Image, FileExtension::Webp),
    media("video/mp4", ModelMediaKind::Video, FileExtension::Mp4),
    media("video/x-m4v", ModelMediaKind::Video, FileExtension::M4v),
    media("video/quicktime", ModelMediaKind::Video, FileExtension::Mov),
    media("video/webm", ModelMediaKind::Video, FileExtension::Webm),
];

const fn media(
    media_type: &'static str,
    kind: ModelMediaKind,
    extension: FileExtension,
) -> ModelMediaType {
    ModelMediaType {
        media_type,
        kind,
        extension,
    }
}

/// The entry of [`MODEL_MEDIA_TYPES`] for `media_type`, if a model can
/// receive it.
pub fn model_media_type_for(media_type: &str) -> Option<&'static ModelMediaType> {
    MODEL_MEDIA_TYPES
        .iter()
        .find(|entry| entry.media_type == media_type)
}

/// Whether `model`'s catalog says it reads `media_type` natively. Unknown
/// support counts as no.
pub fn model_accepts_media_type(model: &Model, media_type: &str) -> bool {
    let Some(entry) = model_media_type_for(media_type) else {
        return false;
    };
    file_extension_support(model.accepted_extensions.as_deref(), entry.extension) == Some(true)
}

/// The media type `bytes` begin with, from their magic numbers, or `None`
/// for anything outside [`MODEL_MEDIA_TYPES`]. Fewer than 12 bytes are never
/// recognized. An ISO media file (`ftyp`) is QuickTime for the brand `qt  `,
/// M4V for a brand starting `M4V`, and MP4 otherwise; an EBML header is WebM,
/// the Matroska format a model reads.
pub fn sniff_model_media_type(bytes: &[u8]) -> Option<&'static ModelMediaType> {
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
    model_media_type_for(media_type)
}
