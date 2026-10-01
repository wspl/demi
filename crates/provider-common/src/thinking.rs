//! How providers whose vendor levels thinking by a token budget read an
//! effort (`models.md` § Thinking and service tiers).

/// The thinking budget each effort names, in tokens.
const EFFORT_BUDGETS: [(&str, u32); 5] = [
    ("low", 4_096),
    ("medium", 16_384),
    ("high", 32_768),
    ("xhigh", 65_536),
    ("max", 98_304),
];

/// The budget of `effort`, in tokens; an effort the ladder does not name
/// thinks like `medium`.
pub fn effort_budget(effort: &str) -> u32 {
    EFFORT_BUDGETS
        .iter()
        .find(|(name, _)| *name == effort)
        .map_or(16_384, |(_, budget)| *budget)
}
