//! What OpenAI publishes of its image models (`images.md` § Vendors), which
//! a ChatGPT subscription and an OpenAI key both read: OpenAI's published
//! IDs, newest first, each with the line its page on
//! developers.openai.com/api/docs/models opens with, the date of its
//! release in OpenAI's API changelog (a dated ID's is its snapshot's, an
//! alias's that of the model it named when the table was made), and the
//! sizes and transparency its image generation guide and changelog give
//! it. The retired `dall-e-*` are left out.

use super::{Aspect, ImageFacts};

/// What OpenAI's flexible sizes take: `gpt-image-2`'s and GPT Image 2.5's
/// in its image generation guide.
macro_rules! flexible {
    ($more:literal) => {
        concat!(
            "any W×H, each edge a multiple of 16 and at most 3840, long:short at most 3:1, 655,360 to 8,294,400 pixels; ",
            $more
        )
    };
}

/// The three sizes of the GPT Image models before `gpt-image-2`, as they
/// are told.
const FIXED: &str = "1024×1024, 1536×1024 or 1024×1536";

/// Those sizes, square first.
const FIXED_SIZES: &[(u32, u32)] = &[(1024, 1024), (1536, 1024), (1024, 1536)];

/// The longer side of the size a shape is sent as to a model that takes
/// any size of it.
const LONG_SIDE: u32 = 1536;

pub(super) const MODELS: &[ImageFacts] = &[
    ImageFacts::new(
        "gpt-image-2.5-sunburst",
        Some("2026-09-08"),
        "Most capable model for image generation and editing",
        flexible!("transparent in PNG or WebP"),
    ),
    ImageFacts::new(
        "gpt-image-2.5-sunburst-2026-09-08",
        Some("2026-09-08"),
        "Dated snapshot of gpt-image-2.5-sunburst",
        flexible!("transparent in PNG or WebP"),
    ),
    ImageFacts::new(
        "gpt-image-2.5-flare",
        Some("2026-09-08"),
        "Fast, high-quality everyday image generation",
        flexible!("transparent in PNG or WebP"),
    ),
    ImageFacts::new(
        "gpt-image-2.5-flare-2026-09-08",
        Some("2026-09-08"),
        "Dated snapshot of gpt-image-2.5-flare",
        flexible!("transparent in PNG or WebP"),
    ),
    ImageFacts::new(
        "gpt-image-2",
        Some("2026-04-21"),
        "State-of-the-art image generation model",
        flexible!("transparent in PNG or WebP, in preview"),
    ),
    ImageFacts::new(
        "gpt-image-2-2026-04-21",
        Some("2026-04-21"),
        "Dated snapshot of gpt-image-2",
        flexible!("transparent in PNG or WebP, in preview"),
    ),
    ImageFacts::new("gpt-image-1.5", Some("2025-12-16"), "Previous image generation model", FIXED).only(FIXED_SIZES),
    ImageFacts::new("gpt-image-1.5-2025-12-16", Some("2025-12-16"), "Dated snapshot of gpt-image-1.5", FIXED).only(FIXED_SIZES),
    ImageFacts::new("chatgpt-image-latest", Some("2025-12-16"), "Previous image model used in ChatGPT", FIXED).only(FIXED_SIZES),
    ImageFacts::new("gpt-image-1-mini", Some("2025-10-06"), "Cost-efficient version of gpt-image-1", FIXED).only(FIXED_SIZES),
    ImageFacts::new("gpt-image-1", Some("2025-04-23"), "Previous image generation model, before gpt-image-1.5", FIXED).only(FIXED_SIZES),
];

/// The `size` an OpenAI-shaped image request sends for `model` and the
/// shape asked: `auto` without one; for a model that takes only some sizes,
/// the one nearest the shape; else the shape at a longer side of 1536.
pub fn size(model: &str, aspect: Option<Aspect>) -> String {
    let Some(aspect) = aspect else {
        return "auto".to_owned();
    };
    let fixed = MODELS.iter().find(|facts| facts.id == model).and_then(|facts| facts.sizes);
    let (width, height) = match fixed {
        Some(sizes) => {
            let asked = f64::from(aspect.width) / f64::from(aspect.height);
            let off = |(width, height): &(u32, u32)| (f64::from(*width) / f64::from(*height) / asked).ln().abs();
            // The table names at least one size for such a model.
            *sizes.iter().min_by(|a, b| off(a).total_cmp(&off(b))).expect("a fixed-size model has sizes")
        }
        None => aspect.size(LONG_SIDE),
    };
    format!("{width}x{height}")
}
