//! What a reply's Markdown names, as the backend reads it.
//!
//! The pictures of the Host a reply shows are named here, once, when its text
//! is complete (`TranscriptLog::complete_tail_text`): the block keeps each by
//! its target, and the page finds a picture's copy by the same target. The
//! target of an image is the one the Markdown parser gives it, which the page
//! reads from its own parser's token with its entities decoded
//! (`packages/web-ui/src/markdown/picture-key.ts`); the two readings are held
//! to one another by
//! `crates/agent-transcript/tests/agent-transcript/fixtures/picture-targets.json`.

use std::sync::LazyLock;

use comrak::{Arena, Options, nodes::NodeValue, parse_document};
use regex::Regex;

/// A URL scheme of two or more letters; a single letter before `:` is a
/// drive.
static SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?i)[a-z][a-z0-9+.-]+:").expect("a valid pattern"));
/// A name with an extension, as the page reads an image's target
/// (`packages/web-ui/src/markdown/filePath.ts`).
static NAME_WITH_DOT: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[^/\\\s]+\.[^/\\\s]+$").expect("a valid pattern"));
/// A line range after a path, as `:12` or `:12-20`.
static LINE_RANGE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r":\d+(?:-\d+)?$").expect("a valid pattern"));
/// Leading `./` components.
static CURRENT_DIR: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\./+").expect("a valid pattern"));

/// Whether `text` starts with a URL scheme, such as `https:`.
pub fn has_scheme(text: &str) -> bool {
    SCHEME.is_match(text)
}

/// How the page's renderer reads a reply's Markdown.
pub fn markdown_options() -> Options<'static> {
    let mut options = Options::default();
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.cjk_friendly_emphasis = true;
    options
}

/// A target as the path it names: without a `file://` scheme, a `:line`
/// suffix, a leading `./` or a trailing slash.
fn normalized(target: &str) -> String {
    let target = target.trim();
    let target = target.strip_prefix("file://").unwrap_or(target);
    let target = LINE_RANGE.replace(target, "");
    let target = CURRENT_DIR.replace(&target, "");
    if target.len() > 1 && target.ends_with('/') {
        target[..target.len() - 1].to_owned()
    } else {
        target.into_owned()
    }
}

/// The path of the Host an image's `target` names, as the reply wrote it
/// without its query, fragment and percent-encoding, which the page reads as
/// a picture of the Host; none for a web image, a `data:` URL or a target
/// that names no file. Relative to the node's working directory, or `~/` from
/// the Host's home.
pub fn picture_path(target: &str) -> Option<String> {
    let trimmed = target.trim();
    let named = normalized(trimmed);
    if named.is_empty() || named.starts_with('#') || SCHEME.is_match(&named) {
        return None;
    }
    let likely = trimmed.ends_with('/') || named.contains(['/', '\\']) || NAME_WITH_DOT.is_match(&named);
    if !likely {
        return None;
    }
    let before_query = trimmed.split(['?', '#']).next().unwrap_or_default();
    let path = normalized(before_query);
    let path = percent_encoding::percent_decode_str(&path)
        .decode_utf8()
        .map_or(path.clone(), |decoded| decoded.into_owned());
    (!path.is_empty()).then_some(path)
}

/// The targets of the images in the Markdown `text`, in order.
fn image_targets(text: &str) -> Vec<String> {
    let arena = Arena::new();
    let root = parse_document(&arena, text, &markdown_options());
    root.descendants()
        .filter_map(|node| match &node.data.borrow().value {
            NodeValue::Image(image) => Some(image.url.clone()),
            _ => None,
        })
        .collect()
}

/// The pictures of the Host `text` shows, each once, by its target, with the
/// path it names.
pub fn named_pictures(text: &str) -> Vec<(String, String)> {
    if !text.contains("![") {
        return Vec::new();
    }
    let mut named: Vec<(String, String)> = Vec::new();
    for target in image_targets(text) {
        if named.iter().any(|(seen, _)| *seen == target) {
            continue;
        }
        if let Some(path) = picture_path(&target) {
            named.push((target, path));
        }
    }
    named
}
