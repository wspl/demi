//! What the model receives of a transcript (`runtime.md` § Replay).

use demi_agent_store::{
    media::{HeldMedia, ModelView},
    testing::test_model,
};
use demi_agent_transcript::{RequestView, replay};
use demi_shared_types::{
    Attachment, BlobRef, Block, BlockId, CompactionBoundaryBlock, CompactionMarkerBlock,
    RedactedThinkingBlock, ThinkingBlock, Timestamp, TurnId, UserBlock, UserContentBlock,
    attachment_tag,
};
use demi_provider_common::{InferenceItem, RequestLimits, UserPart};

/// What the model receives of a message of one text.
fn replayed_text(text: &str) -> String {
    let model = test_model();
    let message = Block::User(UserBlock {
        id: BlockId::try_from("u1").unwrap(),
        turn_id: TurnId::try_from("t1").unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        model: model.clone(),
        content: vec![UserContentBlock::Text { text: text.into() }],
        preamble: None,
    });
    let view = ModelView::of(0, &[message], &HeldMedia::default()).expect("no media to hold");
    let items = replay(&RequestView::new(
        &view,
        &model.model,
        RequestLimits::default(),
    ))
    .items;
    let [InferenceItem::UserMessage { content }] = items.as_slice() else {
        panic!("one message: {items:?}");
    };
    let [UserPart::Text(text)] = content.as_slice() else {
        panic!("one text: {content:?}");
    };
    text.clone()
}

#[test]
fn reasoning_between_the_last_boundary_and_its_marker_is_marked_as_kept_past_a_summary() {
    let model = test_model();
    let id = |value: &str| BlockId::try_from(value).unwrap();
    let thinking = |value: &str| {
        Block::Thinking(ThinkingBlock {
            id: id(value),
            created_at: Timestamp::UNIX_EPOCH,
            model: model.clone(),
            text: value.into(),
            signature: Some(format!("anthropic:{value}")),
        })
    };
    let redacted = Block::RedactedThinking(RedactedThinkingBlock {
        id: id("redacted"),
        created_at: Timestamp::UNIX_EPOCH,
        model: model.clone(),
        data: "anthropic:opaque".into(),
    });
    let boundary = Block::CompactionBoundary(CompactionBoundaryBlock {
        id: id("boundary"),
        created_at: Timestamp::UNIX_EPOCH,
        model: model.clone(),
        summary: "the user asked twice".into(),
        summary_tokens: 5,
    });
    let marker = Block::CompactionMarker(CompactionMarkerBlock {
        id: id("marker"),
        created_at: Timestamp::UNIX_EPOCH,
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

/// A reference names a file on a paired device; the model reads the
/// text that names it, and an attachment record as its tag.
#[test]
fn a_messages_reference_and_attachment_record_reach_the_model_as_their_text() {
    let model = test_model();
    let attachment = Attachment {
        name: "notes.md".into(),
        path: "/home/demi/.demi/attachments/c1/notes.md".into(),
        media_type: "text/markdown".into(),
        size_bytes: 82,
        sha256: BlobRef::of(b"# Notes"),
        snippet: Some("# Notes".into()),
    };
    let reference = "file:///home/demi/notes.md?host=laptop";
    let message = Block::User(UserBlock {
        id: BlockId::try_from("u1").unwrap(),
        turn_id: TurnId::try_from("t1").unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        model: model.clone(),
        content: vec![
            UserContentBlock::Reference {
                reference: reference.into(),
            },
            UserContentBlock::Attachment(attachment.clone()),
        ],
        preamble: None,
    });
    let view = ModelView::of(0, &[message], &HeldMedia::default()).expect("no media to hold");
    assert_eq!(
        replay(&RequestView::new(
            &view,
            &model.model,
            RequestLimits::default()
        ))
        .items,
        [InferenceItem::UserMessage {
            content: vec![
                UserPart::Text(reference.into()),
                UserPart::Text(attachment_tag(&attachment)),
            ]
        }]
    );
}

#[test]
fn a_long_text_keeps_its_ends_and_counts_what_it_left_out_in_scalar_values() {
    let text = format!(
        "{}{}{}",
        "a".repeat(7_999),
        "🙂".repeat(4_002),
        "z".repeat(7_999)
    );
    let bounded = replayed_text(&text);
    let (head, rest) = bounded.split_once("\n\n[... truncated ").unwrap();
    let (count, tail) = rest.split_once(" characters ...]\n\n").unwrap();
    // The cut never splits the emoji: the head ends with one, the tail
    // starts with one, and the count is in scalar values.
    assert_eq!(head, format!("{}🙂", "a".repeat(7_999)));
    assert_eq!(tail, format!("🙂{}", "z".repeat(7_999)));
    assert_eq!(count, "4000");
    assert_eq!(replayed_text(&"x".repeat(16_000)), "x".repeat(16_000));
}
