//! What the model receives past a compaction summary.

use demi_agent_store::{
    media::{HeldMedia, ModelView},
    testing::test_model,
};
use demi_agent_transcript::{RequestView, replay};
use demi_provider_common::{InferenceItem, RequestLimits};
use demi_shared_types::{
    Block, BlockId, CompactionBoundaryBlock, CompactionMarkerBlock, RedactedThinkingBlock,
    ThinkingBlock, Timestamp,
};

#[test]
fn reasoning_between_the_last_boundary_and_its_marker_is_marked_as_kept_past_a_summary() {
    let model = test_model();
    let id = |value: &str| BlockId::try_from(value).unwrap();
    let thinking = |value: &str| {
        Block::Thinking(ThinkingBlock {
            run: None,
            text_length: None,
            entries: Vec::new(),
            id: id(value),
            created_at: Timestamp::UNIX_EPOCH,
            request: None,
            model: model.clone(),
            text: value.into(),
            signature: Some(format!("anthropic:{value}")),
        })
    };
    let redacted = Block::RedactedThinking(RedactedThinkingBlock {
        entries: Vec::new(),
        id: id("redacted"),
        created_at: Timestamp::UNIX_EPOCH,
        request: None,
        model: model.clone(),
        data: "anthropic:opaque".into(),
        run: None,
    });
    let boundary = Block::CompactionBoundary(CompactionBoundaryBlock {
        entries: Vec::new(),
        id: id("boundary"),
        created_at: Timestamp::UNIX_EPOCH,
        request: None,
        model: model.clone(),
        summary: "the user asked twice".into(),
        summary_tokens: 5,
        kept: Vec::new(),
    });
    let marker = Block::CompactionMarker(CompactionMarkerBlock {
        id: id("marker"),
        created_at: Timestamp::UNIX_EPOCH,
        request: None,
        model: model.clone(),
        boundary_id: id("boundary"),
        compacted_tokens: 100,
    });
    let kept = |blocks: &[Block]| -> Vec<(String, bool)> {
        let view = ModelView::of(0, blocks, &HeldMedia::default()).expect("no media to hold");
        replay(&RequestView::new(
            &view,
            &model.model,
            RequestLimits::default(),
            &[],
        ))
        .items
        .into_iter()
        .filter_map(|item| match item {
            InferenceItem::AssistantThinking {
                text,
                kept_past_summary,
                ..
            } => Some((text, kept_past_summary)),
            InferenceItem::AssistantRedactedThinking {
                data,
                kept_past_summary,
                ..
            } => Some((data, kept_past_summary)),
            _ => None,
        })
        .collect()
    };
    let compacted = [
        thinking("summarized"),
        boundary,
        thinking("kept"),
        redacted,
        marker,
        thinking("after"),
    ];
    assert_eq!(
        kept(&compacted),
        [
            ("kept".to_owned(), true),
            ("anthropic:opaque".to_owned(), true),
            ("after".to_owned(), false)
        ]
    );
    // Without a summary, nothing is kept past one.
    assert_eq!(kept(&compacted[5..]), [("after".to_owned(), false)]);
}
