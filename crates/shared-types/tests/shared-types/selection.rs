//! The one conversion of a catalog model into the selection a conversation
//! infers with (`crates/shared-types/src/model.rs`): the types it reads
//! natively and the thinking choices it offers.

use demi_shared_types::{
    FileExtension, ProviderModel, ThinkingCapability, ThinkingSummary, VIDEO_FILE_EXTENSIONS,
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
