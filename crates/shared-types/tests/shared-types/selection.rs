//! The one conversion of a catalog model into the selection a conversation
//! infers with (`models.md` § Request parameters): the model's facts, the
//! types it reads natively and the thinking choices it offers.

use demi_shared_types::{
    ATTACHMENT_FILE_EXTENSIONS, FileExtension, Model, ModelSelection, ProviderModel,
    ThinkingCapability, ThinkingConfig, ThinkingSummary, VIDEO_FILE_EXTENSIONS,
    modality_extensions,
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
        accepted_extensions: None,
        supports_reasoning: None,
        supported_thinking_efforts: None,
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
        accepted_extensions: Some(ATTACHMENT_FILE_EXTENSIONS.to_vec()),
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
    assert_eq!(
        (
            plain.model.context_window,
            plain.thinking,
            plain.service_tier_id
        ),
        (0, None, None)
    );
}

/// Planted defect this catches: a selection that reads its types from
/// anything but the catalog's list, which unknown support would turn into
/// none.
#[test]
fn accepted_types_are_the_catalogs_list_unknown_staying_unknown() {
    let with = |exact: Option<Vec<FileExtension>>| {
        ProviderModel {
            accepted_extensions: exact,
            ..model()
        }
        .selection("p", None, None)
        .model
        .accepted_extensions
    };
    assert_eq!(with(None), None);
    assert_eq!(with(Some(Vec::new())), Some(Vec::new()));
    assert_eq!(
        with(Some(vec![FileExtension::Png])),
        Some(vec![FileExtension::Png])
    );
}

/// Planted defects this catches: a PDF given to every model that reads
/// images, as models.dev's attachment flag did, and video given without its
/// modality.
#[test]
fn input_modalities_give_the_types_kind_by_kind() {
    let images = vec![
        FileExtension::Png,
        FileExtension::Jpg,
        FileExtension::Jpeg,
        FileExtension::Gif,
        FileExtension::Webp,
    ];
    assert_eq!(modality_extensions(["text"]), Vec::new());
    assert_eq!(modality_extensions(["text", "image"]), images);
    let mut with_pdf = images.clone();
    with_pdf.push(FileExtension::Pdf);
    assert_eq!(modality_extensions(["text", "image", "pdf"]), with_pdf);
    assert_eq!(modality_extensions(["video"]), VIDEO_FILE_EXTENSIONS.to_vec());
}

#[test]
fn the_thinking_choices_follow_what_the_catalog_says_of_reasoning() {
    let capabilities = |reasoning, efforts: Option<&[&str]>| {
        ProviderModel {
            supports_reasoning: reasoning,
            supported_thinking_efforts: efforts
                .map(|efforts| efforts.iter().map(|effort| (*effort).to_owned()).collect()),
            ..model()
        }
        .selection("p", None, None)
        .model
        .thinking
    };
    assert_eq!(
        capabilities(Some(false), Some(&["low"])),
        [ThinkingCapability::Disabled {}]
    );
    assert_eq!(capabilities(None, None), []);
    assert_eq!(capabilities(Some(true), Some(&[])), []);
    assert_eq!(
        capabilities(Some(true), Some(&["low", "high"])),
        [ThinkingCapability::Effort {
            efforts: vec!["low".into(), "high".into()],
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
