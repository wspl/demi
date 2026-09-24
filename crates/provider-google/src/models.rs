//! The provider's own model directory (`models.md` § Directories): a list
//! built into the provider, which reading makes no request for.

use demi_core::{ProviderModel, ProviderModelList, Timestamp};

/// The Gemini models a `google` entry offers when it has no configured list
/// and no vendor catalog.
pub(crate) fn directory() -> ProviderModelList {
    let models = vec![
        model("gemini-3.6-flash", "Gemini 3.6 Flash"),
        model("gemini-3.5-flash", "Gemini 3.5 Flash"),
        model("gemini-3.1-pro-preview", "Gemini 3.1 Pro Preview"),
        model("gemini-2.5-flash", "Gemini 2.5 Flash"),
    ];
    ProviderModelList {
        models,
        default_model_id: Some("gemini-3.6-flash".into()),
        warnings: Vec::new(),
        // Built in, so never fetched.
        source_fetched_at: Timestamp::UNIX_EPOCH,
        stale: false,
    }
}

/// A Gemini model with a context of 1,048,576 tokens and an output limit of
/// 65,536: it calls tools, reads images, PDFs and video with its audio
/// natively, and levels its thinking by a token budget that each effort
/// names (`request.rs`).
fn model(id: &str, name: &str) -> ProviderModel {
    let efforts = ["low", "medium", "high", "xhigh", "max"];
    ProviderModel {
        id: id.into(),
        display_name: name.into(),
        description: None,
        context_window: Some(1_048_576),
        output_limit: Some(65_536),
        supports_tools: Some(true),
        supports_attachments: Some(true),
        supports_video: Some(true),
        accepted_extensions: None,
        supports_reasoning: Some(true),
        supported_thinking_efforts: Some(efforts.iter().map(|effort| (*effort).into()).collect()),
        default_thinking_effort: Some("medium".into()),
        can_disable_thinking: None,
        service_tiers: Vec::new(),
        default_service_tier_id: None,
        cost: None,
    }
}
