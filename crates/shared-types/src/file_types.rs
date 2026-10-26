//! The file types the product previews, by extension.
//!
//! The backend serves a file under the media type this table gives it and shows
//! in place only the types marked so. The page picks its viewer from the same
//! table, generated into `@demicodes/protocol`, so the two always agree. An HTML
//! file shown from a Host's files loads them as these types and the web's
//! others.
//!
//! A file is told by its extension, not its content, for three reasons:
//!
//! - The page must choose an element before any byte arrives, since a `<video>`
//!   fetches its own URL.
//! - The type served must match the viewer chosen.
//! - The user's browser must not guess a type on its own (`nosniff`).
//!
//! Every type a model reads natively is in the table and shown in place: one
//! that was not would download, and a player would play it only where the
//! browser guessed its format.

use schemars::JsonSchema;
use serde::Serialize;

/// One media type the product knows files by. Its schema types the table
/// the page receives in `@demicodes/protocol`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
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
pub const PREVIEW_TYPES: [PreviewType; 22] = [
    preview("image/png", &["png"], true),
    preview("image/jpeg", &["jpg", "jpeg"], true),
    preview("image/gif", &["gif"], true),
    preview("image/webp", &["webp"], true),
    preview("image/avif", &["avif"], true),
    preview("image/bmp", &["bmp"], true),
    preview("image/x-icon", &["ico"], true),
    preview("image/svg+xml", &["svg"], true),
    preview("video/mp4", &["mp4"], true),
    // Apple's MP4 variant, the type a model receives M4V bytes as.
    preview("video/x-m4v", &["m4v"], true),
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
    // Shown from its own route (`html_view_media_type`), never served as
    // itself here.
    preview("text/html", &["html", "htm"], false),
];

/// What else an HTML file shown from a Host's files loads, by extension,
/// beside the previewed types: the web's own types that the preview has no
/// viewer for.
const HTML_VIEW_TYPES: [(&str, &[&str]); 9] = [
    ("text/css; charset=utf-8", &["css"]),
    ("text/javascript; charset=utf-8", &["js", "mjs", "cjs"]),
    ("application/json", &["json", "map"]),
    ("application/wasm", &["wasm"]),
    ("text/plain; charset=utf-8", &["txt"]),
    ("application/xml", &["xml"]),
    ("font/woff2", &["woff2"]),
    ("font/woff", &["woff"]),
    ("font/ttf", &["ttf"]),
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
    let extension = extension(path)?;
    PREVIEW_TYPES
        .iter()
        .find(|entry| entry.extensions.contains(&extension.as_str()))
        .map(|entry| entry.media_type)
}

/// The media type an HTML file shown from a Host's files loads the file at
/// `path` as: a previewed type, HTML as UTF-8, or another type of the web's;
/// `None` for any other file.
pub fn html_view_media_type(path: &str) -> Option<&'static str> {
    let extension = extension(path)?;
    let previewed = PREVIEW_TYPES
        .iter()
        .find(|entry| entry.extensions.contains(&extension.as_str()))
        .map(|entry| match entry.media_type {
            "text/html" => "text/html; charset=utf-8",
            media_type => media_type,
        });
    previewed.or_else(|| {
        HTML_VIEW_TYPES
            .iter()
            .find(|(_, extensions)| extensions.contains(&extension.as_str()))
            .map(|(media_type, _)| *media_type)
    })
}

/// The extension of the file at `path`, lowercase: either separator ends a
/// directory, and a name that starts with its only dot has none.
fn extension(path: &str) -> Option<String> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let (stem, extension) = name.rsplit_once('.')?;
    if stem.is_empty() {
        return None;
    }
    Some(extension.to_ascii_lowercase())
}

/// The sandbox an HTML file shown from a Host's files loads in, the
/// `sandbox` policy of its answers and of the frame it shows in: it runs
/// scripts, sends forms, shows `alert`, opens windows that leave the sandbox
/// and downloads, as in a browser tab. Without `allow-same-origin` it never
/// has the product's origin, and without `allow-top-navigation` it never
/// replaces the product's page.
pub const HTML_SANDBOX: &str = "allow-scripts allow-forms allow-modals allow-popups \
    allow-popups-to-escape-sandbox allow-downloads";

/// The content policy an image served in place carries: it runs no script and
/// fetches nothing, in an opaque origin.
pub const IMAGE_CONTENT_POLICY: &str = "default-src 'none'; style-src 'unsafe-inline'; sandbox";

/// Whether the page shows `media_type` in place instead of downloading it.
pub fn shows_in_place(media_type: &str) -> bool {
    PREVIEW_TYPES
        .iter()
        .any(|entry| entry.in_place && entry.media_type == media_type)
}
