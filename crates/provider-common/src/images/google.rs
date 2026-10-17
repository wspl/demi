//! What Google publishes of its Gemini image models (`images.md`
//! § Vendors), which a Gemini API key reads: the models its image
//! generation guide names, newest first, each with what the guide says it
//! is, the date of its release when the Gemini API's release notes give
//! one, and the shapes of `imageConfig.aspectRatio` in its image
//! generation guide (ai.google.dev/gemini-api/docs/image-generation). The
//! listing gives no date, so a date is known only for a model named here.

use super::ImageFacts;

/// The shapes every Gemini image model takes, and the PNG its answers
/// carry.
const SHAPES: &str = "1:1, 3:2, 2:3, 3:4, 4:3, 4:5, 5:4, 9:16, 16:9 or 21:9; sends PNG";

pub(super) const MODELS: &[ImageFacts] = &[
    ImageFacts::new(
        "gemini-nano-banana-2.1",
        None,
        "Nano Banana 2.1: the high-efficiency workhorse for image generation and conversational editing",
        SHAPES,
    ),
    ImageFacts::new(
        "gemini-3.1-flash-lite-image",
        None,
        "Nano Banana 2 Lite: the fastest and cheapest Gemini image model",
        SHAPES,
    ),
    ImageFacts::new(
        "gemini-3.1-flash-image",
        None,
        "Nano Banana 2: the previous-generation high-efficiency workhorse",
        SHAPES,
    ),
    ImageFacts::new(
        "gemini-3-pro-image",
        None,
        "Nano Banana Pro: the premium choice for the most complex visual tasks",
        SHAPES,
    ),
    ImageFacts::new(
        "gemini-2.5-flash-image",
        Some("2025-10-02"),
        "Nano Banana: the legacy pioneer of the series",
        SHAPES,
    ),
];
