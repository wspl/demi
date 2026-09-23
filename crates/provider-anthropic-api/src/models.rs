//! The provider's own model directory (`models.md` § Directories): a list
//! built into the provider, which reading makes no request for.

use demi_core::{ProviderModel, ProviderModelList, Timestamp};

/// The Claude models an `anthropic` entry offers when it has no configured
/// list and no vendor catalog.
pub(crate) fn directory() -> ProviderModelList {
    let wide = ["low", "medium", "high", "xhigh", "max"];
    let narrow = ["low", "medium", "high", "max"];
    let models = vec![
        model("claude-opus-4-8", "Claude Opus 4.8", 128_000, &wide),
        model("claude-opus-4-7", "Claude Opus 4.7", 128_000, &wide),
        model("claude-opus-4-6", "Claude Opus 4.6", 128_000, &narrow),
        model("claude-sonnet-4-6", "Claude Sonnet 4.6", 64_000, &narrow),
        model("claude-fable-5", "Claude Fable 5", 128_000, &wide),
    ];
    ProviderModelList {
        models,
        default_model_id: Some("claude-opus-4-8".into()),
        warnings: Vec::new(),
        // Built in, so never fetched.
        source_fetched_at: Timestamp::UNIX_EPOCH,
        stale: false,
    }
}

/// A Claude model with a one-million-token context: it calls tools, reads
/// images and PDFs but no video, and levels its thinking without turning it
/// off.
fn model(id: &str, name: &str, output_limit: u32, efforts: &[&str]) -> ProviderModel {
    ProviderModel {
        id: id.into(),
        display_name: name.into(),
        description: None,
        context_window: Some(1_000_000),
        output_limit: Some(output_limit),
        supports_tools: Some(true),
        supports_attachments: Some(true),
        supports_video: Some(false),
        accepted_extensions: None,
        supports_reasoning: Some(true),
        supported_thinking_efforts: Some(efforts.iter().map(|effort| (*effort).into()).collect()),
        default_thinking_effort: None,
        can_disable_thinking: Some(false),
        service_tiers: Vec::new(),
        default_service_tier_id: None,
        cost: None,
    }
}
