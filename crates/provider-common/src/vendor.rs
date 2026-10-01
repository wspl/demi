//! A vendor's request requirements (`providers.md` § Vendors from
//! models.dev).

/// A vendor's request requirements, which the backend's vendor policy
/// applies to every model of the vendor. Each protocol's provider reads the
/// fields of its own protocol.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VendorPolicy {
    /// Chat Completions: earlier thinking goes back as `reasoning_content`,
    /// as DeepSeek's thinking mode requires on tool-call continuations.
    /// OpenAI refuses the field.
    pub pass_back_reasoning_content: bool,
    /// Responses: replayed assistant messages carry `status: completed`,
    /// which gateways that validate the full item schema require and relay
    /// gateways refuse.
    pub replay_assistant_status: bool,
    /// Messages: an effort goes as a token budget, the form every
    /// Anthropic-compatible endpoint takes, instead of adaptive thinking at
    /// that effort, which only Anthropic's own API knows.
    pub effort_as_budget: bool,
}
