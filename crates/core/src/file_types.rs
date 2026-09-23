//! The file types the product previews, by extension (`file-previews.md`
//! § Choosing a view). The backend serves a file under the media type this
//! table gives it and shows in place only the types marked so; the page picks
//! its viewer from the same table, generated into `@demicodes/protocol`, so
//! the two always agree.

use serde::Serialize;

/// One media type the product knows files by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewType {
    pub media_type: &'static str,
    /// Lowercase, without the dot.
    pub extensions: &'static [&'static str],
    /// Served as itself for the page to show in place; otherwise the page
    /// downloads it or renders it from its text.
    pub in_place: bool,
}

/// The file-type table.
pub const PREVIEW_TYPES: [PreviewType; 20] = [
    preview("image/png", &["png"], true),
    preview("image/jpeg", &["jpg", "jpeg"], true),
    preview("image/gif", &["gif"], true),
    preview("image/webp", &["webp"], true),
    preview("image/avif", &["avif"], true),
    preview("image/bmp", &["bmp"], true),
    preview("image/x-icon", &["ico"], true),
    preview("image/svg+xml", &["svg"], true),
    preview("video/mp4", &["mp4", "m4v"], true),
    preview("video/webm", &["webm"], true),
    preview("video/quicktime", &["mov"], true),
    preview("audio/mpeg", &["mp3"], true),
    preview("audio/wav", &["wav"], true),
    preview("audio/ogg", &["ogg", "oga", "opus"], true),
    preview("audio/mp4", &["m4a"], true),
    preview("audio/aac", &["aac"], true),
    preview("audio/flac", &["flac"], true),
    preview("audio/webm", &["weba"], true),
    preview("application/pdf", &["pdf"], true),
    // Rendered from its text, never served as itself.
    preview("text/markdown", &["md", "markdown"], false),
];

const fn preview(
    media_type: &'static str,
    extensions: &'static [&'static str],
    in_place: bool,
) -> PreviewType {
    PreviewType {
        media_type,
        extensions,
        in_place,
    }
}

/// The media type the product knows the file at `path` by, from its
/// extension whatever its case, or `None` for a file the table does not
/// name. Either separator ends a directory, since a paired Windows device
/// names its paths with `\`, and a name that starts with its only dot, such
/// as `.png`, has no extension.
pub fn preview_media_type(path: &str) -> Option<&'static str> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let (stem, extension) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    let extension = extension.to_ascii_lowercase();
    PREVIEW_TYPES
        .iter()
        .find(|entry| entry.extensions.contains(&extension.as_str()))
        .map(|entry| entry.media_type)
}

/// Whether the page shows `media_type` in place instead of downloading it.
pub fn shows_in_place(media_type: &str) -> bool {
    PREVIEW_TYPES
        .iter()
        .any(|entry| entry.in_place && entry.media_type == media_type)
}
