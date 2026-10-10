//! What the model receives of a transcript (`runtime.md` § Replay).

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
    Attachment, B64Bytes, BlobRef, Block, BlockId, CommandId, CommandReport,
    CompactionBoundaryBlock, CompactionMarkerBlock, DocumentSource, FileExtension,
    RedactedThinkingBlock, ReportEvent, StoppedBy, ThinkingBlock, Timestamp, ToolCallBlock,
    ToolCallStatus, ToolMediaSource, ToolResultContentBlock, TurnId, UserBlock, UserContentBlock,
    WakeupBlock, WakeupPlacement, attachment_tag,
};

/// What the model receives of a message of one text.
fn replayed_text(text: &str) -> String {
    let model = test_model();
    let message = Block::User(UserBlock {
        entries: Vec::new(),
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
            entries: Vec::new(),
            id: id(value),
            created_at: Timestamp::UNIX_EPOCH,
            model: model.clone(),
            text: value.into(),
            signature: Some(format!("anthropic:{value}")),
        })
    };
    let redacted = Block::RedactedThinking(RedactedThinkingBlock {
        entries: Vec::new(),
        id: id("redacted"),
        created_at: Timestamp::UNIX_EPOCH,
        model: model.clone(),
        data: "anthropic:opaque".into(),
    });
    let boundary = Block::CompactionBoundary(CompactionBoundaryBlock {
        entries: Vec::new(),
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
        entries: Vec::new(),
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
            RequestLimits::default(),
            &[],
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

// A conversation from before `yield` was removed goes on: a vendor may
// refuse a call of a tool its request does not declare.
#[test]
fn a_call_of_a_tool_the_request_no_longer_declares_is_replayed_with_its_result_as_text() {
    let model = test_model();
    let call = |id: &str, tool: &str, input: &str, result: &str| {
        Block::ToolCall(ToolCallBlock {
            entries: Vec::new(),
            id: BlockId::try_from(id).unwrap(),
            created_at: Timestamp::UNIX_EPOCH,
            model: model.clone(),
            tool_use_id: id.into(),
            tool_name: tool.into(),
            input: input.into(),
            status: ToolCallStatus::Completed,
            output: vec![ToolResultContentBlock::Text {
                text: result.into(),
            }],
            view: None,
        })
    };
    let blocks = [
        call("c1", "shell", r#"{"script":"ls"}"#, "status: exited"),
        call(
            "c2",
            "yield",
            r#"{ "durationMs": 600000 }"#,
            "yield scheduled",
        ),
    ];
    let shell = ToolDefinition {
        name: "shell".into(),
        description: String::new(),
        input_schema: serde_json::Map::new(),
    };
    let view = ModelView::of(0, &blocks, &HeldMedia::default()).expect("no media to hold");
    let items = replay(&RequestView::new(
        &view,
        &model.model,
        RequestLimits::default(),
        std::slice::from_ref(&shell),
    ))
    .items;
    let [
        InferenceItem::ToolUse { tool_name, .. },
        InferenceItem::ToolResult { .. },
        InferenceItem::AssistantText { text, .. },
    ] = items.as_slice()
    else {
        panic!("the declared call and the text of the other: {items:?}");
    };
    assert_eq!(tool_name, "shell");
    assert_eq!(text, r#"[called yield {"durationMs":600000}: yield scheduled]"#);
}

/// Planted defects this catches: a tool result's medium replayed by the
/// model's accepted types alone, so a provider that carries no image or no
/// document in a tool result would be sent one; and a document replayed
/// as anything but its bytes and name where the provider carries it.
#[test]
fn each_request_replays_a_tool_results_media_only_where_its_provider_carries_their_kind() {
    let model = model_reading("stub", "m", &[FileExtension::Png, FileExtension::Pdf]);
    let png = demi_agent_store::testing::png(4, 3, 1);
    let pdf = B64Bytes::from(&b"%PDF-1.7\n%%EOF\n"[..]);
    let (png_blob, pdf_blob) = (BlobRef::of(png.as_bytes()), BlobRef::of(pdf.as_bytes()));
    let call = Block::ToolCall(ToolCallBlock {
        entries: Vec::new(),
        id: BlockId::try_from("c1").unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
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

// Pure: a few milliseconds.
#[test]
fn reports_that_arrived_together_reach_the_model_as_their_text_one_paragraph_each() {
    let model = test_model();
    let report = |command: &str, title: &str, event: ReportEvent, output: &str| CommandReport {
        command_id: CommandId::try_from(command).unwrap(),
        title: title.into(),
        event,
        output: output.into(),
        media: Vec::new(),
    };
    let wakeup = Block::Wakeup(WakeupBlock {
        id: BlockId::try_from("w1").unwrap(),
        turn_id: TurnId::try_from("t1").unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        model: model.clone(),
        placement: WakeupPlacement::NewTurn,
        reports: vec![
            report(
                "17",
                "Run the test suite",
                ReportEvent::Running {
                    running_ms: 300_000,
                    idle_ms: 290_000,
                    interval_ms: 300_000,
                },
                "",
            ),
            report(
                "17",
                "Run the test suite",
                ReportEvent::Ended { exit_code: Some(1) },
                "FAIL auth.test.ts\n1 failed",
            ),
            report("18", "Start the dev server", ReportEvent::Stopped { by: Some(StoppedBy::User) }, ""),
            report("19", "", ReportEvent::Stopped { by: Some(StoppedBy::Agent { number: 2 }) }, ""),
            report(
                "20",
                "Watch the files",
                ReportEvent::Lost {
                    reason: "Demi was upgraded and the Host's runner replaced itself".into(),
                },
                "",
            ),
        ],
        entries: Vec::new(),
    });
    let view = ModelView::of(0, &[wakeup], &HeldMedia::default()).expect("no media to hold");
    let items = replay(&RequestView::new(&view, &model.model, RequestLimits::default(), &[])).items;
    let [InferenceItem::UserMessage { content }] = items.as_slice() else {
        panic!("one message: {items:?}");
    };
    let [UserPart::Text(text)] = content.as_slice() else {
        panic!("one text: {content:?}");
    };
    assert_eq!(
        text,
        "Command 17 (Run the test suite) is still running after 5m; no output for 4m50s.\n\
         output: (empty)\n\
         It reports every 5m; change that with demi shell status 17 --interval <duration>, or with --resident to hear only of its end.\n\
         \n\
         Command 17 (Run the test suite) ended with exit code 1.\n\
         output:\n\
         FAIL auth.test.ts\n\
         1 failed\n\
         \n\
         Command 18 (Start the dev server) was stopped by the user.\n\
         output: (empty)\n\
         \n\
         Command 19 was stopped by agent 2.\n\
         output: (empty)\n\
         \n\
         Command 20 (Watch the files) was lost: Demi was upgraded and the Host's runner replaced itself. Start it again if it is still needed.\n\
         output: (empty)"
    );
}
