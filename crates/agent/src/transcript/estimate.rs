//! Token estimates (`compaction.md` § Token estimates): compaction decides
//! from these, not from a tokenizer, and the latest usage a provider
//! reported keeps the context estimate close to the provider's own count.

use std::borrow::Cow;

use demi_core::{
    B64Bytes, Block, DocumentSource, ModelMediaKind, ToolResultContentBlock, UserContentBlock,
};
use demi_provider::{InferenceItem, MediaBytes, Medium, ResultPart, UserPart};

use super::{RESUME_TEXT, WAKEUP_TEXT, gone_text, replay::RequestView, replay_start};

/// An image with its bytes weighs at least this many tokens, as does an
/// image the model fetches by URL.
const IMAGE_TOKENS: u64 = 1_600;
/// Bytes per token of an image with its bytes.
const IMAGE_BYTES_PER_TOKEN: u64 = 1_000;
/// Bytes per token of a document.
const DOCUMENT_BYTES_PER_TOKEN: u64 = 4;

/// A text's estimate: its UTF-8 bytes divided by 4, rounded up, so `hello`
/// and `你好` are both 2 tokens.
pub fn text_tokens(text: &str) -> u64 {
    byte_count(text.len()).div_ceil(4)
}

/// A block's estimate in `request`: the estimate of its text plus the
/// weight of its media, each as the request carries it (`compaction.md`
/// § Block estimates): with its bytes, weighed by them, or as the text that
/// names it, which counts as text.
pub fn block_tokens(block: &Block, request: &RequestView) -> u64 {
    let (text, media) = block_estimate(block, request);
    text_tokens(&text) + media
}

/// The estimate of `request`, the next request of its model. It is anchored
/// on the latest usage a provider reported after the last compaction: that
/// usage plus the estimates of the blocks after it. There is no anchor when
/// a compaction came after that response, when the last compaction has no
/// marker yet, or when the usage is larger than the model's context window,
/// which one request's usage cannot be; then the estimate is the sum of the
/// blocks from the last compaction boundary on.
pub fn context_tokens(request: &RequestView) -> u64 {
    let blocks = request.blocks();
    let window = request.model().context_window;
    let anchor =
        usage_anchor(blocks).filter(|(_, tokens)| window == 0 || *tokens <= u64::from(window));
    let estimate = |block: &Block| block_tokens(block, request);
    if let Some((index, tokens)) = anchor {
        return tokens + blocks[index + 1..].iter().map(estimate).sum::<u64>();
    }
    blocks[replay_start(blocks)..].iter().map(estimate).sum()
}

/// What a request weighs as its vendor receives it (`compaction.md`
/// § Request size).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct RequestSize {
    /// The base64 length of the media it carries, and the UTF-8 length of its
    /// system prompt and texts; the vendor's format adds its keys and escapes.
    pub bytes: u64,
    /// The image blocks it carries, in messages and tool results alike.
    pub images: u64,
}

/// The size of a request with `system_prompt` and `items`, which replay made
/// for the request's model, so what it could not take is text already.
pub fn request_size(system_prompt: &str, items: &[InferenceItem]) -> RequestSize {
    let mut size = RequestSize {
        bytes: byte_count(system_prompt.len()),
        images: 0,
    };
    for item in items {
        match item {
            InferenceItem::UserMessage { content } | InferenceItem::UserSteer { content } => {
                for part in content {
                    size.add_content(part);
                }
            }
            InferenceItem::AssistantText { text, .. } => size.add_text(text),
            InferenceItem::AssistantThinking {
                text, signature, ..
            } => {
                size.add_text(text);
                size.add_text(signature.as_deref().unwrap_or_default());
            }
            InferenceItem::AssistantRedactedThinking { data, .. } => size.add_text(data),
            InferenceItem::ToolUse {
                tool_use_id,
                tool_name,
                input,
                ..
            } => {
                size.add_text(tool_use_id);
                size.add_text(tool_name);
                size.add_text(&input.to_string());
            }
            InferenceItem::ToolResult {
                tool_use_id,
                output,
                ..
            } => {
                size.add_text(tool_use_id);
                for part in output {
                    match part {
                        ResultPart::Text(text) => size.add_text(text),
                        ResultPart::Image(bytes) => {
                            size.images += 1;
                            size.bytes += bytes.data.base64_len();
                        }
                        ResultPart::Video(bytes) => size.bytes += bytes.data.base64_len(),
                    }
                }
            }
        }
    }
    size
}

impl RequestSize {
    fn add_text(&mut self, text: &str) {
        self.bytes += byte_count(text.len());
    }

    fn add_content(&mut self, part: &UserPart) {
        match part {
            UserPart::Text(text) => self.add_text(text),
            UserPart::Image(medium) => {
                self.images += 1;
                self.add_medium(medium);
            }
            UserPart::Video(medium) => self.add_medium(medium),
            UserPart::Document { bytes, .. } => self.bytes += bytes.data.base64_len(),
        }
    }

    fn add_medium(&mut self, medium: &Medium) {
        match medium {
            Medium::Bytes(bytes) => self.bytes += bytes.data.base64_len(),
            Medium::Url(url) => self.add_text(url),
        }
    }
}

/// The latest response block with usage above zero, and that usage, unless
/// a compaction invalidated it.
fn usage_anchor(blocks: &[Block]) -> Option<(usize, u64)> {
    let boundary = blocks.iter().rev().find_map(|block| match block {
        Block::CompactionBoundary(boundary) => Some(&boundary.id),
        _ => None,
    });
    if let Some(boundary) = boundary {
        let marked = blocks.iter().any(
            |block| matches!(block, Block::CompactionMarker(marker) if &marker.boundary_id == boundary),
        );
        // A summary inserted before retained responses changes the history
        // their usage measured, until its marker says what it replaced.
        if !marked {
            return None;
        }
    }
    for (index, block) in blocks.iter().enumerate().rev() {
        match block {
            Block::CompactionBoundary(_) | Block::CompactionMarker(_) => return None,
            Block::Response(response) => {
                let usage = response.usage;
                let tokens = usage.input_tokens
                    + usage.output_tokens
                    + usage.cache_read_tokens
                    + usage.cache_write_tokens;
                if tokens > 0 {
                    return Some((index, tokens));
                }
            }
            _ => {}
        }
    }
    None
}

/// The text a block's estimate counts, and the weight of its media.
fn block_estimate(block: &Block, request: &RequestView) -> (String, u64) {
    let text = match block {
        Block::User(user) => return content_estimate(&user.content, request),
        Block::Steer(steer) => return content_estimate(&steer.content, request),
        Block::ToolCall(call) => {
            let mut lines = vec![
                Cow::Borrowed(call.tool_name.as_str()),
                Cow::Borrowed(call.input.as_str()),
            ];
            let mut media = 0;
            for part in &call.output {
                let (line, weight) = result_estimate(part, request);
                lines.push(line);
                media += weight;
            }
            return (lines.join("\n"), media);
        }
        Block::Wakeup(_) => WAKEUP_TEXT.to_owned(),
        Block::Context(context) => context.text.clone(),
        Block::AgentMessage(receipt) => {
            serde_json::to_string(&receipt.message).expect("an agent message serializes to JSON")
        }
        Block::Resume(_) => RESUME_TEXT.to_owned(),
        Block::Thinking(thinking) => thinking.text.clone(),
        Block::RedactedThinking(redacted) => redacted.data.clone(),
        Block::Text(text) => text.text.clone(),
        Block::Response(response) => {
            serde_json::to_string(&response.usage).expect("token usage serializes to JSON")
        }
        Block::Error(error) => error.message.clone(),
        Block::Abort(_) => "aborted".to_owned(),
        Block::CompactionBoundary(boundary) => boundary.summary.clone(),
        Block::CompactionMarker(marker) => marker.compacted_tokens.to_string(),
    };
    (text, 0)
}

/// A message's content: one line per part, and the weight of its media.
fn content_estimate(content: &[UserContentBlock], request: &RequestView) -> (String, u64) {
    let mut lines = Vec::with_capacity(content.len());
    let mut media = 0;
    for part in content {
        let (line, weight) = content_part_estimate(part, request);
        lines.push(line);
        media += weight;
    }
    (lines.join("\n"), media)
}

/// A message's part: its line, and the weight of the medium the request
/// carries. An image or a video is its URL or media type, a document
/// `<fileName> <mediaType>`, and a medium the request carries as text that
/// text; videos weigh nothing.
fn content_part_estimate<'a>(
    part: &'a UserContentBlock,
    request: &RequestView,
) -> (Cow<'a, str>, u64) {
    match part {
        UserContentBlock::Text { text } => (Cow::Borrowed(text.as_str()), 0),
        UserContentBlock::Reference { reference } => (Cow::Borrowed(reference.as_str()), 0),
        UserContentBlock::Attachment(attachment) => (
            Cow::Owned(format!("{} {}", attachment.name, attachment.path)),
            0,
        ),
        UserContentBlock::Image { source } => match request.medium(ModelMediaKind::Image, source) {
            Ok(Medium::Bytes(MediaBytes { data, media_type })) => {
                (Cow::Owned(media_type), image_weight(&data))
            }
            Ok(Medium::Url(url)) => (Cow::Owned(url), IMAGE_TOKENS),
            Err(text) => (Cow::Owned(text), 0),
        },
        UserContentBlock::Video { source } => match request.medium(ModelMediaKind::Video, source) {
            Ok(Medium::Bytes(MediaBytes { media_type, .. })) => (Cow::Owned(media_type), 0),
            Ok(Medium::Url(url)) => (Cow::Owned(url), 0),
            Err(text) => (Cow::Owned(text), 0),
        },
        UserContentBlock::Document { source } => match request.document(source) {
            Ok(MediaBytes { data, media_type }) => {
                let DocumentSource::Ref { file_name, .. } = source;
                (
                    Cow::Owned(format!("{file_name} {media_type}")),
                    byte_count(data.len()).div_ceil(DOCUMENT_BYTES_PER_TOKEN),
                )
            }
            Err(text) => (Cow::Owned(text), 0),
        },
    }
}

/// A tool result's part: its line, and the weight of the medium the
/// request carries. A medium is its media type, or the text the request
/// carries in its place, and a medium that is gone is its text.
fn result_estimate<'a>(
    part: &'a ToolResultContentBlock,
    request: &RequestView,
) -> (Cow<'a, str>, u64) {
    let (kind, source) = match part {
        ToolResultContentBlock::Text { text } => return (Cow::Borrowed(text.as_str()), 0),
        ToolResultContentBlock::Gone {
            kind,
            media_type,
            cause,
        } => return (Cow::Owned(gone_text(*kind, media_type, cause)), 0),
        ToolResultContentBlock::Image { source } => (ModelMediaKind::Image, source),
        ToolResultContentBlock::Video { source } => (ModelMediaKind::Video, source),
    };
    match request.tool_medium(kind, source) {
        Ok(MediaBytes { data, media_type }) => {
            let weight = match kind {
                ModelMediaKind::Image => image_weight(&data),
                ModelMediaKind::Video => 0,
            };
            (Cow::Owned(media_type), weight)
        }
        Err(text) => (Cow::Owned(text), 0),
    }
}

/// The weight of an image the request carries with its bytes.
fn image_weight(data: &B64Bytes) -> u64 {
    IMAGE_TOKENS.max(byte_count(data.len()).div_ceil(IMAGE_BYTES_PER_TOKEN))
}
fn byte_count(bytes: usize) -> u64 {
    u64::try_from(bytes).expect("a length fits in 64 bits")
}

#[cfg(test)]
mod tests {
    use demi_core::{
        BlobRef, CompactionBoundaryBlock, CompactionMarkerBlock, FileExtension, MediaSource, Model,
        ResponseBlock, TextBlock, Timestamp, TokenUsage, ToolCallBlock, ToolCallStatus,
        ToolMediaSource, TurnId, UserBlock,
    };
    use demi_provider::RequestLimits;

    use super::*;
    use crate::{
        store::media::{HeldMedia, ModelView},
        testing::{model_reading, test_model},
    };

    fn user(id: &str, text: &str) -> Block {
        Block::User(UserBlock {
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
        context_tokens(&RequestView::new(&view, &model, RequestLimits::default()))
    }

    #[test]
    fn the_latest_usage_anchors_the_estimate_unless_it_exceeds_the_window() {
        let answer = Block::Text(TextBlock {
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
            id: "u".try_into().unwrap(),
            turn_id: TurnId::try_from("turn").unwrap(),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            content: vec![
                UserContentBlock::Image {
                    source: MediaSource::Ref {
                        r#ref: BlobRef::of(&image),
                        media_type: "image/png".into(),
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
            let request = RequestView::new(&view, model, limits);
            blocks.each_ref().map(|block| block_tokens(block, &request))
        };
        let unlimited = RequestLimits::default();
        let unread = |kind: &str, name: &str| {
            format!("[{kind}:{name}, not sent: the model does not accept it]")
        };

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
}
