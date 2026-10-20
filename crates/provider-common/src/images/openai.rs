//! The size an OpenAI-shaped image request sends, a ChatGPT
//! subscription's and an OpenAI key's alike.

use super::{Aspect, nearest_size};
use crate::media_catalog::Catalog;

/// The longer side of the size a shape is sent as to a model that takes
/// any size of it.
const LONG_SIDE: u32 = 1536;

/// The `size` an OpenAI-shaped image request sends for OpenAI's `model` and the
/// shape asked.
///
/// - `auto` without a shape.
/// - For a model the catalog says takes only some sizes, the one nearest the
///   shape.
/// - Otherwise the shape at a longer side of 1536.
pub fn size(catalog: &Catalog, model: &str, aspect: Option<Aspect>) -> String {
    let Some(aspect) = aspect else {
        return "auto".to_owned();
    };
    let (width, height) = match nearest_size(catalog, crate::media::MediaVendor::OpenAi.id(), model, aspect) {
        Some(size) => (size.width, size.height),
        None => aspect.size(LONG_SIDE),
    };
    format!("{width}x{height}")
}
