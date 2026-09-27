//! Token estimates (`compaction.md` § Token estimates): compaction decides
//! from these, not from a tokenizer, and the latest usage a provider
//! reported keeps the context estimate close to the provider's own count.

use std::borrow::Cow;

use demi_core::{
    BlobRef, Block, DocumentSource, MediaSource, ToolMediaSource, ToolResultContentBlock,
    UserContentBlock,
};
use demi_provider::{InferenceItem, Medium, ResultPart, UserPart};

use super::{RESUME_TEXT, WAKEUP_TEXT, gone_text, replay_start};
use crate::store::media::{Held, HeldMedia, missing_text};

/// An image with its bytes weighs at least this many tokens, as does an
/// image the model fetches by URL, and one nothing is held for.
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

/// A block's estimate: the estimate of its text plus the weight of its
/// media, each with what `media` holds for it (`compaction.md` § Block
/// estimates). A medium whose blob is missing counts as its text; one that
/// nothing is held for, as in blocks read from frames, weighs what an
/// image by URL weighs, and a document nothing.
pub fn block_tokens(block: &Block, media: &HeldMedia) -> u64 {
    text_tokens(&block_text(block, media)) + media_tokens(block, media)
}

/// The estimate of the next request over `blocks`, whose media weigh what
/// `media` holds for them. It is anchored on the latest usage a provider
/// reported after the last compaction: that usage plus the estimates of the
/// blocks after it. There is no anchor when a compaction came after that
/// response, when the last compaction has no marker yet, or when the usage
/// is larger than `context_window`, which one request's usage cannot be;
/// then the estimate is the sum of the blocks from the last compaction
/// boundary on.
pub fn context_tokens(blocks: &[Block], media: &HeldMedia, context_window: Option<u32>) -> u64 {
    let anchor = usage_anchor(blocks).filter(|(_, tokens)| {
        context_window.is_none_or(|window| window == 0 || *tokens <= u64::from(window))
    });
    let estimate = |block: &Block| block_tokens(block, media);
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

/// The text a block's estimate counts.
fn block_text(block: &Block, media: &HeldMedia) -> String {
    match block {
        Block::User(user) => content_text(&user.content, media),
        Block::Steer(steer) => content_text(&steer.content, media),
        Block::Wakeup(_) => WAKEUP_TEXT.to_owned(),
        Block::Context(context) => context.text.clone(),
        Block::AgentMessage(receipt) => {
            serde_json::to_string(&receipt.message).expect("an agent message serializes to JSON")
        }
        Block::Resume(_) => RESUME_TEXT.to_owned(),
        Block::Thinking(thinking) => thinking.text.clone(),
        Block::RedactedThinking(redacted) => redacted.data.clone(),
        Block::Text(text) => text.text.clone(),
        Block::ToolCall(call) => {
            let mut lines = vec![
                Cow::Borrowed(call.tool_name.as_str()),
                Cow::Borrowed(call.input.as_str()),
            ];
            lines.extend(call.output.iter().map(|part| match part {
                ToolResultContentBlock::Text { text } => Cow::Borrowed(text.as_str()),
                ToolResultContentBlock::Image {
                    source: ToolMediaSource::Ref { r#ref, media_type },
                } => medium_line("image", r#ref, Cow::Borrowed(media_type), media),
                ToolResultContentBlock::Video {
                    source: ToolMediaSource::Ref { r#ref, media_type },
                } => medium_line("video", r#ref, Cow::Borrowed(media_type), media),
                ToolResultContentBlock::Gone {
                    kind,
                    media_type,
                    cause,
                } => Cow::Owned(gone_text(*kind, media_type, cause)),
            }));
            lines.join("\n")
        }
        Block::Response(response) => {
            serde_json::to_string(&response.usage).expect("token usage serializes to JSON")
        }
        Block::Error(error) => error.message.clone(),
        Block::Abort(_) => "aborted".to_owned(),
        Block::CompactionBoundary(boundary) => boundary.summary.clone(),
        Block::CompactionMarker(marker) => marker.compacted_tokens.to_string(),
    }
}

/// One line per part of a message's content.
fn content_text(content: &[UserContentBlock], media: &HeldMedia) -> String {
    content
        .iter()
        .map(|part| match part {
            UserContentBlock::Text { text } => Cow::Borrowed(text.as_str()),
            UserContentBlock::Image {
                source: MediaSource::Ref { r#ref, media_type },
            } => medium_line("image", r#ref, Cow::Borrowed(media_type), media),
            UserContentBlock::Video {
                source: MediaSource::Ref { r#ref, media_type },
            } => medium_line("video", r#ref, Cow::Borrowed(media_type), media),
            UserContentBlock::Image {
                source: MediaSource::Url { url },
            }
            | UserContentBlock::Video {
                source: MediaSource::Url { url },
            } => Cow::Borrowed(url.as_str()),
            UserContentBlock::Document {
                source:
                    DocumentSource::Ref {
                        r#ref,
                        media_type,
                        file_name,
                    },
            } => medium_line(
                "document",
                r#ref,
                Cow::Owned(format!("{file_name} {media_type}")),
                media,
            ),
            UserContentBlock::Reference { reference } => Cow::Borrowed(reference.as_str()),
            UserContentBlock::Attachment(attachment) => {
                Cow::Owned(format!("{} {}", attachment.name, attachment.path))
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The line of the medium of `kind` that `blob` names: `line`, or the
/// medium's text when its blob is missing.
fn medium_line<'a>(
    kind: &str,
    blob: &BlobRef,
    line: Cow<'a, str>,
    media: &HeldMedia,
) -> Cow<'a, str> {
    match media.get(blob) {
        Some(Held::Missing) => Cow::Owned(missing_text(kind, blob)),
        Some(Held::Bytes(_)) | None => line,
    }
}

/// The weight of a block's images and documents; videos weigh nothing.
fn media_tokens(block: &Block, media: &HeldMedia) -> u64 {
    match block {
        Block::User(user) => user
            .content
            .iter()
            .map(|part| content_media_tokens(part, media))
            .sum(),
        Block::Steer(steer) => steer
            .content
            .iter()
            .map(|part| content_media_tokens(part, media))
            .sum(),
        Block::ToolCall(call) => call
            .output
            .iter()
            .map(|part| match part {
                ToolResultContentBlock::Image {
                    source: ToolMediaSource::Ref { r#ref, .. },
                } => image_weight(r#ref, media),
                ToolResultContentBlock::Text { .. }
                | ToolResultContentBlock::Video { .. }
                | ToolResultContentBlock::Gone { .. } => 0,
            })
            .sum(),
        _ => 0,
    }
}

fn content_media_tokens(part: &UserContentBlock, media: &HeldMedia) -> u64 {
    match part {
        UserContentBlock::Image {
            source: MediaSource::Ref { r#ref, .. },
        } => image_weight(r#ref, media),
        UserContentBlock::Image {
            source: MediaSource::Url { .. },
        } => IMAGE_TOKENS,
        UserContentBlock::Document {
            source: DocumentSource::Ref { r#ref, .. },
        } => match media.get(r#ref) {
            Some(Held::Bytes(data)) => byte_count(data.len()).div_ceil(DOCUMENT_BYTES_PER_TOKEN),
            Some(Held::Missing) | None => 0,
        },
        UserContentBlock::Text { .. }
        | UserContentBlock::Video { .. }
        | UserContentBlock::Reference { .. }
        | UserContentBlock::Attachment(_) => 0,
    }
}

/// The weight of the image `blob` names: by its bytes, nothing when its
/// blob is missing, since it counts as its text, and what an image by URL
/// weighs when nothing is held for it.
fn image_weight(blob: &BlobRef, media: &HeldMedia) -> u64 {
    match media.get(blob) {
        Some(Held::Bytes(data)) => {
            IMAGE_TOKENS.max(byte_count(data.len()).div_ceil(IMAGE_BYTES_PER_TOKEN))
        }
        Some(Held::Missing) => 0,
        None => IMAGE_TOKENS,
    }
}

fn byte_count(bytes: usize) -> u64 {
    u64::try_from(bytes).expect("a length fits in 64 bits")
}

#[cfg(test)]
mod tests {
    use demi_core::{
        B64Bytes, CompactionBoundaryBlock, CompactionMarkerBlock, ResponseBlock, TextBlock,
        Timestamp, TokenUsage, TurnId, UserBlock,
    };

    use super::*;
    use crate::testing::test_model;

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
        let none = HeldMedia::default();
        assert_eq!(context_tokens(&blocks, &none, None), 1_384);
        blocks.push(user("u2", &"y".repeat(4_000)));
        assert_eq!(context_tokens(&blocks, &none, None), 1_384 + 1_000);
        // A usage above the window is a provider reporting something else.
        blocks.push(response("r2", 2_000_000));
        let unanchored = context_tokens(&blocks, &none, Some(1_000_000));
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
        assert!(context_tokens(&blocks, &HeldMedia::default(), None) < 10_000);
    }

    #[test]
    fn images_and_documents_weigh_by_their_held_bytes() {
        let image = B64Bytes::from(vec![0; 3_000_000]);
        let document = B64Bytes::from(vec![1; 40_000]);
        let user = Block::User(UserBlock {
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
        let mut held = HeldMedia::default();
        held.hold(BlobRef::of(&image), image);
        held.hold(BlobRef::of(&document), document);
        let text = text_tokens("image/png\nhttps://example.com/a.png\ndoc.pdf application/pdf");
        assert_eq!(block_tokens(&user, &held), text + 3_000 + 1_600 + 10_000);
        // Held by reference only, as blocks read from frames are: an image
        // weighs what one by URL weighs, and a document nothing.
        assert_eq!(
            block_tokens(&user, &HeldMedia::default()),
            text + 1_600 + 1_600
        );
    }
}
