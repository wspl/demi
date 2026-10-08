//! Token estimates (`compaction.md` § Token estimates): compaction decides
//! from these, not from a tokenizer, and the latest usage a provider
//! reported keeps the context estimate close to the provider's own count.

use std::borrow::Cow;

use demi_provider_common::{InferenceItem, MediaBytes, Medium, ResultPart, UserPart};
use demi_shared_types::{
    B64Bytes, Block, DocumentSource, Model, ModelMediaKind, ToolResultContentBlock,
    UserContentBlock,
};

use super::{
    gone_text,
    replay::{RESUME_TEXT, RequestView, wakeup_text},
    replay_start,
};

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

/// Where the estimate of the next request starts weighing blocks
/// (`compaction.md` § Context estimate): the latest usage a provider
/// reported, and the first block after its `response`; or no usage and the
/// last compaction boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextAnchor {
    /// The reported usage the estimate starts from; zero without one.
    pub tokens: u64,
    /// The index, in the blocks the anchor was found in, of the first block
    /// whose estimate is added: only these blocks' media weigh.
    pub from: usize,
}

/// The anchor of the next request of `model` over the replayed `blocks`.
/// There is none when a compaction came after the latest response, when the
/// last compaction has no marker yet, or when the usage is larger than the
/// model's own context window, which one request's usage cannot be.
pub fn context_anchor(blocks: &[Block], model: &Model) -> ContextAnchor {
    let window = model.context_window;
    match usage_anchor(blocks).filter(|(_, tokens)| window == 0 || *tokens <= u64::from(window)) {
        Some((index, tokens)) => ContextAnchor {
            tokens,
            from: index + 1,
        },
        None => ContextAnchor {
            tokens: 0,
            from: replay_start(blocks),
        },
    }
}

/// The estimate of `request`, the next request of its model: its
/// [`context_anchor`] plus the estimates of the blocks from the anchor on.
pub fn context_tokens(request: &RequestView) -> u64 {
    let blocks = request.blocks();
    let anchor = context_anchor(blocks, request.model());
    anchor.tokens + blocks_tokens(&blocks[anchor.from..], request)
}

/// The estimates of `blocks`, summed, each as `request` carries it.
pub fn blocks_tokens(blocks: &[Block], request: &RequestView) -> u64 {
    blocks.iter().map(|block| block_tokens(block, request)).sum()
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
        Block::Wakeup(wakeup) => wakeup_text(wakeup).into_owned(),
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
        ToolResultContentBlock::Gone { kind, cause, .. } => {
            return (Cow::Owned(gone_text(*kind, cause)), 0);
        }
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
