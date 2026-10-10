//! The token estimates compaction decides by (`compaction.md` § Estimates).

use demi_agent_store::{
    media::{HeldMedia, ModelView},
    testing::{model_reading, test_model},
};
use demi_agent_transcript::{
    RequestView,
    estimate::{block_tokens, context_tokens, text_tokens},
};
use demi_provider_common::RequestLimits;
use demi_shared_types::{
    B64Bytes, BlobRef, Block, CompactionBoundaryBlock, CompactionMarkerBlock, DocumentSource,
    FileExtension, MediaSource, Model, ResponseBlock, TextBlock, Timestamp, TokenUsage,
    ToolCallBlock, ToolCallStatus, ToolMediaSource, ToolResultContentBlock, TurnId, UserBlock,
    UserContentBlock,
};

fn user(id: &str, text: &str) -> Block {
    Block::User(UserBlock {
        entries: Vec::new(),
        id: id.try_into().unwrap(),
        turn_id: TurnId::try_from("turn").unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        content: vec![UserContentBlock::Text { text: text.into() }],
        preamble: Some("not counted".into()),
    })
}

fn response(id: &str, input_tokens: u64) -> Block {
    Block::Response(ResponseBlock {
        id: id.try_into().unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        usage: TokenUsage {
            input_tokens,
            output_tokens: 50,
            cache_read_tokens: 100,
            cache_write_tokens: 0,
        },
    })
}

/// The estimate of the next request over `blocks`, which hold no media,
/// to a model whose context window is `window`.
fn estimate(blocks: &[Block], window: u32) -> u64 {
    let view = ModelView::of(0, blocks, &HeldMedia::default()).expect("no media to hold");
    let mut model = test_model().model;
    model.context_window = window;
    let limits = RequestLimits {
        tool_results: demi_provider_common::ToolResultKinds::ALL,
        ..RequestLimits::default()
    };
    context_tokens(&RequestView::new(&view, &model, limits, &[]))
}

#[test]
fn the_latest_usage_anchors_the_estimate_unless_it_exceeds_the_window() {
    let answer = Block::Text(TextBlock {
        entries: Vec::new(),
        id: "t".try_into().unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        text: "reply".into(),
        forkable: true,
    });
    let mut blocks = vec![
        user("u1", &"x".repeat(40_000)),
        answer,
        response("r1", 1_234),
    ];
    // Anchored: the reported usage replaces the much larger estimate of
    // the text.
    assert_eq!(estimate(&blocks, 1_000_000), 1_384);
    blocks.push(user("u2", &"y".repeat(4_000)));
    assert_eq!(estimate(&blocks, 1_000_000), 1_384 + 1_000);
    // A usage above the window is a provider reporting something else.
    blocks.push(response("r2", 2_000_000));
    let unanchored = estimate(&blocks, 1_000_000);
    assert!(unanchored > 10_000 && unanchored < 20_000, "{unanchored}");
}

#[test]
fn a_compaction_after_the_latest_response_leaves_no_anchor() {
    let boundary = Block::CompactionBoundary(CompactionBoundaryBlock {
        entries: Vec::new(),
        id: "b".try_into().unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        summary: "short summary".into(),
        summary_tokens: 4,
    });
    let marker = Block::CompactionMarker(CompactionMarkerBlock {
        id: "m".try_into().unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        boundary_id: "b".try_into().unwrap(),
        compacted_tokens: 2_000,
    });
    let blocks = vec![
        boundary,
        user("u1", &"x".repeat(8_000)),
        response("r1", 999_999),
        marker,
    ];
    // Without the anchor: the boundary's summary, the user text, the
    // usage's JSON and the marker's count.
    assert!(estimate(&blocks, 1_000_000) < 10_000);
}

/// A medium weighs what the request to its model carries: its bytes, by
/// them, or the text that names it, as text. A model that does not read
/// a type, or a vendor whose requests it would take more than half of,
/// gets the text.
#[test]
fn a_medium_weighs_what_the_request_to_its_model_carries() {
    let image = B64Bytes::from(vec![0; 3_000_000]);
    let document = B64Bytes::from(vec![1; 40_000]);
    let screenshot = B64Bytes::from(vec![2; 1_800_000]);
    let message = Block::User(UserBlock {
        entries: Vec::new(),
        id: "u".try_into().unwrap(),
        turn_id: TurnId::try_from("turn").unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        content: vec![
            UserContentBlock::Image {
                source: MediaSource::Ref {
                    r#ref: BlobRef::of(&image),
                    media_type: "image/png".into(),
                    width: None,
                    height: None,
                },
            },
            UserContentBlock::Image {
                source: MediaSource::Url {
                    url: "https://example.com/a.png".into(),
                },
            },
            UserContentBlock::Document {
                source: DocumentSource::Ref {
                    r#ref: BlobRef::of(&document),
                    media_type: "application/pdf".into(),
                    file_name: "doc.pdf".into(),
                },
            },
        ],
        preamble: None,
    });
    let call = Block::ToolCall(ToolCallBlock {
        entries: Vec::new(),
        id: "c".try_into().unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        tool_use_id: "call-1".into(),
        tool_name: "shoot".into(),
        input: "{}".into(),
        status: ToolCallStatus::Completed,
        output: vec![ToolResultContentBlock::Image {
            source: ToolMediaSource::Ref {
                r#ref: BlobRef::of(&screenshot),
                media_type: "image/png".into(),
                width: None,
                height: None,
            },
        }],
        view: None,
    });
    let mut held = HeldMedia::default();
    for bytes in [&image, &document, &screenshot] {
        held.hold(BlobRef::of(bytes), bytes.clone());
    }
    let blocks = [message, call];
    let view = ModelView::of(0, &blocks, &held).unwrap();
    let reads = model_reading("stub", "reads", &[FileExtension::Png, FileExtension::Pdf]).model;
    let blind = model_reading("stub", "blind", &[]).model;
    let weigh = |model: &Model, limits| {
        let request = RequestView::new(&view, model, limits, &[]);
        blocks.each_ref().map(|block| block_tokens(block, &request))
    };
    let unlimited = RequestLimits {
        tool_results: demi_provider_common::ToolResultKinds::ALL,
        ..RequestLimits::default()
    };
    let unread =
        |kind: &str, name: &str| format!("[{kind}:{name}, not sent: the model does not accept it]");

    // Each with its bytes.
    assert_eq!(
        weigh(&reads, unlimited),
        [
            text_tokens("image/png\nhttps://example.com/a.png\ndoc.pdf application/pdf")
                + 3_000
                + 1_600
                + 10_000,
            text_tokens("shoot\n{}\nimage/png") + 1_800
        ]
    );
    // A model that reads neither type: the texts, and the image it
    // fetches by URL.
    assert_eq!(
        weigh(&blind, unlimited),
        [
            text_tokens(&format!(
                "{}\nhttps://example.com/a.png\n{}",
                unread("image", "image/png"),
                unread("document", "doc.pdf")
            )) + 1_600,
            text_tokens(&format!("shoot\n{{}}\n{}", unread("image", "image/png")))
        ]
    );
    // Requests of 5 MB: the 3 MB image takes 4 MB as base64, over half,
    // and the screenshot 2.4 MB.
    let small = RequestLimits {
        body_bytes: Some(5_000_000),
        images: None,
        tool_results: demi_provider_common::ToolResultKinds::ALL,
    };
    let too_large = "[image:image/png, not sent: too large for the model's requests]";
    assert_eq!(
        weigh(&reads, small),
        [
            text_tokens(&format!(
                "{too_large}\nhttps://example.com/a.png\ndoc.pdf application/pdf"
            )) + 1_600
                + 10_000,
            text_tokens("shoot\n{}\nimage/png") + 1_800
        ]
    );
}
