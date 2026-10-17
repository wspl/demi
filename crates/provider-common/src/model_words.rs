//! The words of a model's ID or name, which the small model's tiers
//! (`models.md` § Small model) and a key's image models (`images.md`
//! § Vendors) match whole.

/// The lowercase words of `text`, split at `-`, `_`, `.`, `/` and spaces.
pub fn model_words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(['-', '_', '.', '/', ' '])
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}
