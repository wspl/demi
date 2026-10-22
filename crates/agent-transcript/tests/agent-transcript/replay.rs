//! What the model receives of a transcript.

use demi_agent_store::{
    media::{HeldMedia, ModelView},
    testing::{model_reading, test_model},
};
use demi_agent_transcript::{RequestView, replay};
use demi_provider_common::{
    InferenceItem, MediaBytes, RequestLimits, ResultPart, ToolDefinition, ToolResultKinds,
    UserPart,
};
use demi_shared_types::{
    B64Bytes, BlobRef, Block, BlockId, CompactionBoundaryBlock, CompactionMarkerBlock,
    DocumentSource, FileExtension, RedactedThinkingBlock, ThinkingBlock, Timestamp,
    ToolCallBlock, ToolCallStatus, ToolMediaSource, ToolResultContentBlock, TurnId, UserBlock,
    UserContentBlock,
};

/// What the model receives of a message of one text.
fn replayed_text(text: &str) -> String {
    let model = test_model();
    let message = Block::User(UserBlock {
        comments: Vec::new(),
        entries: Vec::new(),
        id: BlockId::try_from("u1").unwrap(),
        turn_id: TurnId::try_from("t1").unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        request: None,
        model: model.clone(),
        content: vec![UserContentBlock::Text { text: text.into() }],
        preamble: None,
        automation: None,
    });
    let view = ModelView::of(0, &[message], &HeldMedia::default()).expect("no media to hold");
    let items = replay(&RequestView::new(
        &view,
        &model.model,
        RequestLimits::default(),
        &[],
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

/// Planted defects this catches:
///
/// - a tool result's medium replayed by the model's accepted types alone, so a
///   provider that carries no image or no document in a tool result would be
///   sent one;
/// - a document replayed as anything but its bytes and name where the provider
///   carries it.
#[test]
fn each_request_replays_a_tool_results_media_only_where_its_provider_carries_their_kind() {
    let model = model_reading("stub", "m", &[FileExtension::Png, FileExtension::Pdf]);
    let png = demi_agent_store::testing::png(4, 3, 1);
    let pdf = B64Bytes::from(&b"%PDF-1.7\n%%EOF\n"[..]);
    let (png_blob, pdf_blob) = (BlobRef::of(png.as_bytes()), BlobRef::of(pdf.as_bytes()));
    let call = Block::ToolCall(ToolCallBlock {
        run: None,
        entries: Vec::new(),
        ended_at: None,
        id: BlockId::try_from("c1").unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        request: None,
        model: model.clone(),
        tool_use_id: "c1".into(),
        tool_name: "shell".into(),
        input: r#"{"script":"demi file view a.png r.pdf"}"#.into(),
        status: ToolCallStatus::Completed,
        output: vec![
            ToolResultContentBlock::Image {
                source: ToolMediaSource::Ref {
                    r#ref: png_blob.clone(),
                    media_type: "image/png".into(),
                    width: Some(4),
                    height: Some(3),
                },
            },
            ToolResultContentBlock::Document {
                source: DocumentSource::Ref {
                    r#ref: pdf_blob.clone(),
                    media_type: "application/pdf".into(),
                    file_name: "document-2.pdf".into(),
                },
            },
        ],
        view: None,
    });
    let mut held = HeldMedia::default();
    held.hold(png_blob, png.clone());
    held.hold(pdf_blob, pdf.clone());
    let blocks = [call];
    let view = ModelView::of(0, &blocks, &held).expect("the media are held");
    let shell = ToolDefinition {
        name: "shell".into(),
        description: String::new(),
        input_schema: serde_json::Map::new(),
    };
    let result = |tool_results| {
        let limits = RequestLimits {
            tool_results,
            ..RequestLimits::default()
        };
        let items = replay(&RequestView::new(
            &view,
            &model.model,
            limits,
            std::slice::from_ref(&shell),
        ))
        .items;
        items
            .into_iter()
            .find_map(|item| match item {
                InferenceItem::ToolResult { output, .. } => Some(output),
                _ => None,
            })
            .expect("the call's result")
    };
    assert_eq!(
        result(ToolResultKinds::IMAGES_AND_DOCUMENTS),
        [
            ResultPart::Image(MediaBytes {
                data: png,
                media_type: "image/png".into(),
            }),
            ResultPart::Document {
                bytes: MediaBytes {
                    data: pdf,
                    media_type: "application/pdf".into(),
                },
                file_name: "document-2.pdf".into(),
            },
        ]
    );
    assert_eq!(
        result(ToolResultKinds::NONE),
        [
            ResultPart::Text("[image:image/png, not sent: the model does not accept it]".into()),
            ResultPart::Text(
                "[document:document-2.pdf, not sent: the model does not accept it]".into()
            ),
        ]
    );
}

