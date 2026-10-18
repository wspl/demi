//! Token estimates (`compaction.md` § Token estimates): compaction decides
//! from these, not from a tokenizer, and the latest usage a provider
//! reported keeps the context estimate close to the provider's own count.

use std::borrow::Cow;

use demi_provider_common::{MediaBytes, Medium, ToolDefinition};
use demi_shared_types::{
    MessageComment,
    B64Bytes, Block, DocumentSource, INSTRUCTIONS_SOURCE, Model, ModelMediaKind, ResponseBlock,
    ToolResultContentBlock, UserContentBlock, comments_content,
};

use super::{
    COMPACTION_NOTE, gone_text,
    replay::{RequestView, bound_text, resume_text},
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

/// The estimate of a tool's definition: its JSON's, as a request declares
/// it.
pub fn tool_tokens(tool: &ToolDefinition) -> u64 {
    let json = serde_json::json!({
        "name": tool.name,
        "description": tool.description,
        "input_schema": tool.input_schema,
    });
    text_tokens(&json.to_string())
}

/// The estimate of what a request carries before its history: its
/// `system_prompt` and `tools`, each its text's (`compaction.md` § Context
/// estimate, rule 5).
pub fn prompt_tokens(system_prompt: &str, tools: &[ToolDefinition]) -> u64 {
    text_tokens(system_prompt) + tools.iter().map(tool_tokens).sum::<u64>()
}

/// Where the estimate of the next request starts weighing blocks
/// (`compaction.md` § Context estimate): the latest usage a provider
/// reported, and the first block after its `response`; or no usage and the
/// last compaction boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ContextAnchor {
    /// Whether a reported usage anchors the estimate.
    pub anchored: bool,
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
            anchored: true,
            tokens,
            from: index + 1,
        },
        None => ContextAnchor {
            anchored: false,
            tokens: 0,
            from: replay_start(blocks),
        },
    }
}

/// The estimate of `request`, the next request of its model, with
/// `system_prompt`: its [`context_anchor`] plus the estimates of the blocks
/// from the anchor on; without an anchor, the estimate of the system prompt
/// and the tools in its place, so a request just after a compaction is not
/// read as the history alone.
pub fn context_tokens(request: &RequestView, system_prompt: &str) -> u64 {
    let blocks = request.blocks();
    let anchor = context_anchor(blocks, request.model());
    anchor_tokens(&anchor, system_prompt, request.tools())
        + blocks_tokens(&blocks[anchor.from..], request)
}

/// What the estimate starts from at `anchor`: the usage it holds, or
/// without one the estimate of `system_prompt` and `tools`.
pub fn anchor_tokens(anchor: &ContextAnchor, system_prompt: &str, tools: &[ToolDefinition]) -> u64 {
    if anchor.anchored {
        anchor.tokens
    } else {
        prompt_tokens(system_prompt, tools)
    }
}

/// The estimates of `blocks`, summed, each as `request` carries it.
pub fn blocks_tokens(blocks: &[Block], request: &RequestView) -> u64 {
    blocks.iter().map(|block| block_tokens(block, request)).sum()
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
            // A response without usage anchors nothing: the estimate goes
            // on from an earlier one, weighing the blocks after it.
            Block::Response(ResponseBlock { usage: Some(usage), .. }) => {
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
        Block::User(user) => return with_comments(content_estimate(&user.content, request), &user.comments, request),
        Block::Steer(steer) => return with_comments(content_estimate(&steer.content, request), &steer.comments, request),
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
        Block::Wakeup(wakeup) => {
            let mut lines = vec![bound_text(&crate::reports_text(&wakeup.reports)).into_owned()];
            let mut media = 0;
            for part in wakeup.reports.iter().flat_map(|report| &report.media) {
                let (line, weight) = result_estimate(part, request);
                lines.push(line.into_owned());
                media += weight;
            }
            return (lines.join("\n"), media);
        }
        // The instructions are sent whole (`compaction.md` § Text bounds).
        Block::Context(context) if context.source == INSTRUCTIONS_SOURCE => context.text.clone(),
        Block::Context(context) => bound_text(&context.text).into_owned(),
        Block::AgentMessage(receipt) => {
            serde_json::to_string(&receipt.message).expect("an agent message serializes to JSON")
        }
        Block::Resume(resume) => resume_text(resume),
        Block::Thinking(thinking) => match &thinking.signature {
            Some(_) => thinking.text.clone(),
            None => bound_text(&thinking.text).into_owned(),
        },
        Block::RedactedThinking(redacted) => redacted.data.clone(),
        Block::Text(text) => bound_text(&text.text).into_owned(),
        Block::Response(response) => {
            serde_json::to_string(&response.usage).expect("token usage serializes to JSON")
        }
        Block::Error(error) => error.message.clone(),
        Block::Abort(_) => "aborted".to_owned(),
        // The model receives nothing of it (`compaction.md` § Text bounds).
        Block::AccountSwitch(_) => String::new(),
        // The note, the messages it kept and the summary, as the model
        // reads them.
        Block::CompactionBoundary(boundary) => {
            let mut lines = vec![COMPACTION_NOTE.to_owned()];
            let mut media = 0;
            for kept in &boundary.kept {
                let (text, weight) = content_estimate(&kept.content, request);
                lines.push(text);
                media += weight;
            }
            lines.push(bound_text(&boundary.summary).into_owned());
            return (lines.join("\n"), media);
        }
        Block::CompactionMarker(marker) => marker.compacted_tokens.to_string(),
    };
    (text, 0)
}

/// The estimate of a message's content in `request`: one line per part,
/// and the weight of its media.
pub fn content_tokens(content: &[UserContentBlock], request: &RequestView) -> u64 {
    let (text, media) = content_estimate(content, request);
    text_tokens(&text) + media
}

/// A message's or a steer's estimate with its comments before its content:
/// their text parts, each bounded as replay bounds it, and their pictures.
fn with_comments((text, media): (String, u64), comments: &[MessageComment], request: &RequestView) -> (String, u64) {
    let lead = comments_content(comments);
    if lead.is_empty() {
        return (text, media);
    }
    let (lead_text, lead_media) = content_estimate(&lead, request);
    (format!("{lead_text}\n{text}"), media + lead_media)
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
        UserContentBlock::Text { text } => (bound_text(text), 0),
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
        ToolResultContentBlock::Text { text } => return (bound_text(text), 0),
        ToolResultContentBlock::Gone { kind, cause, .. } => {
            return (Cow::Owned(gone_text(*kind, cause)), 0);
        }
        ToolResultContentBlock::Image { source } => (ModelMediaKind::Image, source),
        ToolResultContentBlock::Video { source } => (ModelMediaKind::Video, source),
        ToolResultContentBlock::Document { source } => {
            return match request.tool_document(source) {
                Ok(MediaBytes { data, media_type }) => {
                    let DocumentSource::Ref { file_name, .. } = source;
                    (
                        Cow::Owned(format!("{file_name} {media_type}")),
                        byte_count(data.len()).div_ceil(DOCUMENT_BYTES_PER_TOKEN),
                    )
                }
                Err(text) => (Cow::Owned(text), 0),
            };
        }
    };
    match request.tool_medium(kind, source) {
        Ok(MediaBytes { data, media_type }) => {
            let weight = if kind == ModelMediaKind::Image {
                image_weight(&data)
            } else {
                0
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
