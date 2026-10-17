//! The size an OpenAI-shaped image request sends, a ChatGPT
//! subscription's and an OpenAI key's alike (`images.md` § Vendors).

use super::Aspect;
use crate::media_catalog::Catalog;

/// The longer side of the size a shape is sent as to a model that takes
/// any size of it.
const LONG_SIDE: u32 = 1536;

/// The `size` an OpenAI-shaped image request sends for OpenAI's `model`
/// and the shape asked: `auto` without one; for a model the catalog says
/// takes only some sizes, the one nearest the shape; else the shape at a
/// longer side of 1536.
pub fn size(catalog: &Catalog, model: &str, aspect: Option<Aspect>) -> String {
    let Some(aspect) = aspect else {
        return "auto".to_owned();
    };
    let fixed = catalog
        .model(super::ImageVendor::OpenAi.id(), model)
        .and_then(|model| catalog.image(model))
        .and_then(|image| image.sizes.as_deref());
    let asked = f64::from(aspect.width) / f64::from(aspect.height);
    let off = |width: u32, height: u32| (f64::from(width) / f64::from(height) / asked).ln().abs();
    let nearest = fixed.and_then(|sizes| {
        sizes
            .iter()
            .min_by(|a, b| off(a.width, a.height).total_cmp(&off(b.width, b.height)))
    });
    let (width, height) = match nearest {
        Some(size) => (size.width, size.height),
        None => aspect.size(LONG_SIDE),
    };
    format!("{width}x{height}")
}
