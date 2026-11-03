//! The words of a model's ID or name, which the small model's tiers
//! (`crates/provider-registry/src/small_model.rs`) and a key's image models match whole,
//! and whether one model's ID names a snapshot of another's.

/// Whether `model` is a dated or numbered snapshot of `of`, the ID a vendor
/// lists or answers with for an alias: `of`, `-` and one of
///
/// - eight digits, Anthropic's (`claude-opus-4-5-20251101`);
/// - a date or four digits, OpenAI's (`gpt-5-2025-08-07`, `gpt-4-0613`);
/// - three digits, or a month and a day or a year, Google's
///   (`gemini-2.0-flash-001`, `gemini-2.5-flash-preview-05-20`,
///   `gemini-2.5-flash-preview-09-2025`).
///
/// Another model whose ID only begins with `of`, such as `claude-opus-5-5`
/// for `claude-opus-5`, is none.
pub fn is_snapshot(model: &str, of: &str) -> bool {
    let Some(suffix) = model.strip_prefix(of).and_then(|rest| rest.strip_prefix('-')) else {
        return false;
    };
    let digits = |text: &str, count: usize| text.len() == count && text.bytes().all(|byte| byte.is_ascii_digit());
    let parts: Vec<&str> = suffix.split('-').collect();
    match parts.as_slice() {
        [number] => digits(number, 8) || digits(number, 4) || digits(number, 3),
        [month, day_or_year] => digits(month, 2) && (digits(day_or_year, 2) || digits(day_or_year, 4)),
        [year, month, day] => digits(year, 4) && digits(month, 2) && digits(day, 2),
        _ => false,
    }
}

/// The lowercase words of `text`, split at `-`, `_`, `.`, `/` and spaces.
pub fn model_words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(['-', '_', '.', '/', ' '])
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}
