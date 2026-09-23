//! The provider's own model directory (`models.md` § Directories): a list
//! built into the provider, which reading makes no request for.

use demi_core::{ProviderModel, ProviderModelList, ServiceTier, Timestamp};

/// The models an `openai` entry offers when it has no configured list and no
/// vendor catalog.
pub(crate) fn directory() -> ProviderModelList {
    let models = vec![
        model("gpt-5.5", "GPT-5.5", 272_000, true, true),
        model("gpt-5.4", "GPT-5.4", 272_000, true, true),
        model("gpt-5.4-mini", "GPT-5.4-Mini", 272_000, true, false),
        model("gpt-5.3-codex-spark", "GPT-5.3-Codex-Spark", 128_000, false, false),
    ];
    ProviderModelList {
        models,
        default_model_id: Some("gpt-5.5".into()),
        warnings: Vec::new(),
        // Built in, so never fetched.
        source_fetched_at: Timestamp::UNIX_EPOCH,
        stale: false,
    }
}

/// A GPT model: it calls tools, reads images and PDFs when `attachments`,
/// levels its thinking from `low` to `xhigh`, has no model-specific output
/// limit, and offers the `priority` tier as Fast when `fast`.
fn model(id: &str, name: &str, context_window: u32, attachments: bool, fast: bool) -> ProviderModel {
    let efforts = ["low", "medium", "high", "xhigh"];
    let service_tiers = if fast {
        vec![ServiceTier {
            id: "priority".into(),
            label: "Fast".into(),
            description: Some("1.5x speed, increased usage".into()),
            fast: true,
        }]
    } else {
        Vec::new()
    };
    ProviderModel {
        id: id.into(),
        display_name: name.into(),
        description: None,
        context_window: Some(context_window),
        output_limit: None,
        supports_tools: Some(true),
        supports_attachments: Some(attachments),
        supports_video: None,
        accepted_extensions: None,
        supports_reasoning: Some(true),
        supported_thinking_efforts: Some(efforts.iter().map(|effort| (*effort).into()).collect()),
        default_thinking_effort: None,
        can_disable_thinking: None,
        service_tiers,
        default_service_tier_id: None,
        cost: None,
    }
}
