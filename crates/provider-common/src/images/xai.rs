//! What xAI publishes of its image models (`images.md` § Vendors), which a
//! Grok subscription and an xAI key both read, and how xAI's image API
//! refuses a shape.

use super::ImageFacts;

/// What every xAI image model takes: the shapes of `aspect_ratio` in xAI's
/// images API reference, and the JPEG its answers carry.
const SHAPES: &str = "1:1, 3:4, 4:3, 9:16, 16:9, 2:3, 3:2, 9:19.5, 19.5:9, 9:20, 20:9, 1:2, 2:1, 21:9 or 5:2; sends JPEG";

/// The image models, newest first: xAI's published IDs. xAI's model pages
/// carry no one-line description, so each says what xAI publishes of it:
/// the price on x.ai/api/imagine, Image 2.0's announcement, the
/// retirement in its release notes, and the aliases on
/// docs.x.ai/developers/models. A date is the dated alias's, or Image
/// 2.0's announcement's. The retired `grok-2-image*` are left out.
pub(super) const MODELS: &[ImageFacts] = &[
    ImageFacts::new(
        "grok-imagine-image-2.0",
        Some("2026-08-07"),
        "Precise image generation and editing, built for real creative work",
        SHAPES,
    ),
    ImageFacts::new(
        "grok-imagine-image-quality",
        Some("2026-04-03"),
        "Retires on November 2, 2026; grok-imagine-image-2.0 covers its uses at a lower price",
        SHAPES,
    ),
    ImageFacts::new("grok-imagine-image-quality-20260403", Some("2026-04-03"), "Alias of grok-imagine-image-quality", SHAPES),
    ImageFacts::new("grok-imagine-image-quality-latest", Some("2026-04-03"), "Alias of grok-imagine-image-quality", SHAPES),
    ImageFacts::new("grok-imagine-image-pro", Some("2026-04-03"), "Alias of grok-imagine-image-quality", SHAPES),
    ImageFacts::new("grok-imagine-image", Some("2026-03-02"), "Lowest-cost image model, from $0.02 an image", SHAPES),
    ImageFacts::new("grok-imagine-image-2026-03-02", Some("2026-03-02"), "Alias of grok-imagine-image", SHAPES),
];

/// The shapes xAI's image API takes, as its refusal of `aspect_ratio` lists
/// them: ``aspect_ratio: unknown variant `4:5`, expected one of `1:1`,
/// `3:4`, …, `auto` at line 1 column 304``. The whole list is read, each
/// entry quoted and the last after `, ` or ` or `; only the W:H shapes are
/// kept, as `auto` is no shape `--aspect` asks for. None for any other
/// refusal or a list not read whole, which is then told in xAI's own
/// words.
fn shapes_taken(message: &str) -> Option<Vec<String>> {
    let (_, rest) = message.split_once("aspect_ratio: unknown variant `")?;
    let (_, mut rest) = rest.split_once("`, expected one of ")?;
    let mut takes = Vec::new();
    loop {
        let (entry, after) = rest.strip_prefix('`')?.split_once('`')?;
        if entry.contains(':') {
            takes.push(entry.to_owned());
        } else if entry != "auto" {
            return None;
        }
        match after.strip_prefix(", ").or_else(|| after.strip_prefix(" or ")) {
            Some(next) => rest = next,
            // The list ends where no separator follows: at the message's
            // end or its position, `at line …`.
            None if after.is_empty() || after.starts_with(" at ") => break,
            None => return None,
        }
    }
    (!takes.is_empty()).then_some(takes)
}

/// `answer` with a refusal of the shape asked for told as the shapes xAI
/// named, so a shape xAI adds needs no change (`images.md` § Commands).
pub fn with_shapes(mut answer: super::ImageAnswer) -> super::ImageAnswer {
    if let Err(super::ImageFailure::Refused { message, code }) = &answer.result
        && let Some(takes) = shapes_taken(message)
    {
        answer.result = Err(super::ImageFailure::Shape { takes, code: code.clone() });
    }
    answer
}
