//! The binary types a model can receive natively (`models.md` § Accepted
//! attachment types). The set is closed: bytes are known by their magic
//! numbers or not at all, never by guessing from a name; the command wire
//! recognizes them (`demi_command_protocol::sniff_media_type`), so that a
//! runner tells a medium without linking this crate.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{FileExtension, Model, file_extension_support};

/// Whether a model reads a medium as an image, a video or a document.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ModelMediaKind {
    Image,
    Video,
    Document,
}

impl ModelMediaKind {
    /// The word a text names the kind by: `image`, `video` or `document`.
    pub fn name(self) -> &'static str {
        match self {
            Self::Image => "image",
            Self::Video => "video",
            Self::Document => "document",
        }
    }
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
pub const MODEL_MEDIA_TYPES: [ModelMediaType; 9] = [
    media("image/png", ModelMediaKind::Image, FileExtension::Png),
    media("image/jpeg", ModelMediaKind::Image, FileExtension::Jpeg),
    media("image/gif", ModelMediaKind::Image, FileExtension::Gif),
    media("image/webp", ModelMediaKind::Image, FileExtension::Webp),
    media("video/mp4", ModelMediaKind::Video, FileExtension::Mp4),
    media("video/x-m4v", ModelMediaKind::Video, FileExtension::M4v),
    media("video/quicktime", ModelMediaKind::Video, FileExtension::Mov),
    media("video/webm", ModelMediaKind::Video, FileExtension::Webm),
    media("application/pdf", ModelMediaKind::Document, FileExtension::Pdf),
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
