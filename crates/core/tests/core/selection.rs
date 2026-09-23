//! The one conversion of a catalog model into the selection a conversation
//! infers with (`models.md` § Request parameters): the model's facts, the
//! types it reads natively and the thinking choices it offers.

use demi_core::{
    ATTACHMENT_FILE_EXTENSIONS, FileExtension, Model, ModelSelection, ProviderModel, ThinkingCapability,
    ThinkingConfig, ThinkingSummary, VIDEO_FILE_EXTENSIONS,
};

/// A model whose catalog states nothing but its name and limits.
fn model() -> ProviderModel {
    ProviderModel {
        id: "m-1".into(),
        display_name: "Model One".into(),
        description: None,
        context_window: Some(200_000),
        output_limit: None,
        supports_tools: None,
        supports_attachments: None,
        supports_video: None,
        accepted_extensions: None,
        supports_reasoning: None,
        supported_thinking_efforts: None,
        default_thinking_effort: None,
        can_disable_thinking: None,
        service_tiers: Vec::new(),
        default_service_tier_id: None,
        cost: None,
    }
}

#[test]
fn a_catalog_model_becomes_a_selection_with_its_facts_and_the_choices_made() {
    let catalog = ProviderModel {
        output_limit: Some(8_000),
        supports_attachments: Some(true),
        ..model()
    };
    let thinking = ThinkingConfig::Effort {
        effort: "high".into(),
        summary: None,
    };
    let selection = catalog.selection("p", Some(thinking.clone()), Some("priority".into()));
    assert_eq!(
        selection,
        ModelSelection {
            provider_id: "p".into(),
            model: Model {
                id: "m-1".into(),
                name: "Model One".into(),
                context_window: 200_000,
                input_limit: None,
                output_limit: Some(8_000),
                thinking: Vec::new(),
                accepted_extensions: Some(ATTACHMENT_FILE_EXTENSIONS.to_vec()),
            },
            thinking: Some(thinking),
            service_tier_id: Some("priority".into()),
        }
    );
    let unknown = ProviderModel {
        context_window: None,
        ..model()
    };
    let plain = unknown.selection("p", None, None);
    assert_eq!((plain.model.context_window, plain.thinking, plain.service_tier_id), (0, None, None));
}

#[test]
fn accepted_types_come_from_the_catalog_or_from_its_flags() {
    let with = |attachments, video, exact: Option<Vec<FileExtension>>| {
        ProviderModel {
            supports_attachments: attachments,
            supports_video: video,
            accepted_extensions: exact,
            ..model()
        }
        .selection("p", None, None)
        .model
        .accepted_extensions
    };
    let attachments = ATTACHMENT_FILE_EXTENSIONS.to_vec();
    let both: Vec<FileExtension> = ATTACHMENT_FILE_EXTENSIONS.into_iter().chain(VIDEO_FILE_EXTENSIONS).collect();
    // Unknown support stays unknown; known support adds its types.
    assert_eq!(with(None, None, None), None);
    assert_eq!(with(None, Some(false), None), None);
    assert_eq!(with(Some(false), None, None), Some(Vec::new()));
    assert_eq!(with(Some(true), None, None), Some(attachments.clone()));
    assert_eq!(with(Some(true), Some(false), None), Some(attachments));
    // Only a model known to read video gets the video types.
    assert_eq!(with(Some(true), Some(true), None), Some(both));
    assert_eq!(with(Some(false), Some(true), None), Some(VIDEO_FILE_EXTENSIONS.to_vec()));
    assert_eq!(with(None, Some(true), None), Some(VIDEO_FILE_EXTENSIONS.to_vec()));
    // The exact list a catalog states wins over its flags, an empty one too.
    assert_eq!(with(Some(true), None, Some(vec![FileExtension::Png])), Some(vec![FileExtension::Png]));
    assert_eq!(with(Some(true), Some(true), Some(Vec::new())), Some(Vec::new()));
}

#[test]
fn the_thinking_choices_follow_what_the_catalog_says_of_reasoning() {
    let capabilities = |reasoning, efforts: Option<&[&str]>, default: Option<&str>| {
        ProviderModel {
            supports_reasoning: reasoning,
            supported_thinking_efforts: efforts.map(|efforts| efforts.iter().map(|effort| (*effort).to_owned()).collect()),
            default_thinking_effort: default.map(str::to_owned),
            ..model()
        }
        .selection("p", None, None)
        .model
        .thinking
    };
    assert_eq!(capabilities(Some(false), Some(&["low"]), None), [ThinkingCapability::Disabled {}]);
    assert_eq!(capabilities(None, None, None), []);
    assert_eq!(capabilities(Some(true), Some(&[]), None), []);
    assert_eq!(
        capabilities(Some(true), Some(&["low", "high"]), Some("high")),
        [ThinkingCapability::Effort {
            efforts: vec!["low".into(), "high".into()],
            default_effort: Some("high".into()),
            summaries: vec![
                ThinkingSummary::Auto,
                ThinkingSummary::Concise,
                ThinkingSummary::Detailed,
                ThinkingSummary::Off,
                ThinkingSummary::On,
            ],
            default_summary: None,
        }]
    );
}
