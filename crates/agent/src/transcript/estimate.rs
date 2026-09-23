//! Token estimates (`compaction.md` § Token estimates): compaction decides
//! from these, not from a tokenizer, and the latest usage a provider
//! reported keeps the context estimate close to the provider's own count.

use demi_core::{
    Block, DocumentSource, MediaSource, ToolMediaSource, ToolResultContentBlock, UserContentBlock,
};

use super::{RESUME_TEXT, WAKEUP_TEXT};

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

/// A block's estimate: the estimate of its text plus the weight of its
/// media.
pub fn block_tokens(block: &Block) -> u64 {
    text_tokens(&block_text(block)) + media_tokens(block)
}

/// The estimate of the next request over `blocks`. It is anchored on the
/// latest usage a provider reported after the last compaction: that usage
/// plus the estimates of the blocks after it. There is no anchor when a
/// compaction came after that response, when the last compaction has no
/// marker yet, or when the usage is larger than `context_window`, which one
/// request's usage cannot be; then the estimate is the sum of the blocks
/// from the last compaction boundary on.
pub fn context_tokens(blocks: &[Block], context_window: Option<u32>) -> u64 {
    let anchor = usage_anchor(blocks).filter(|(_, tokens)| {
        context_window.is_none_or(|window| window == 0 || *tokens <= u64::from(window))
    });
    if let Some((index, tokens)) = anchor {
        return tokens + blocks[index + 1..].iter().map(block_tokens).sum::<u64>();
    }
    let start = blocks
        .iter()
        .rposition(|block| matches!(block, Block::CompactionBoundary(_)))
        .unwrap_or(0);
    blocks[start..].iter().map(block_tokens).sum()
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
fn block_text(block: &Block) -> String {
    match block {
        Block::User(user) => content_text(&user.content),
        Block::Steer(steer) => content_text(&steer.content),
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
            let mut lines = vec![call.tool_name.as_str(), call.input.as_str()];
            lines.extend(call.output.iter().map(|part| match part {
                ToolResultContentBlock::Text { text } => text.as_str(),
                ToolResultContentBlock::Image { source }
                | ToolResultContentBlock::Video { source } => match source {
                    ToolMediaSource::Binary { media_type, .. }
                    | ToolMediaSource::Ref { media_type, .. } => media_type.as_str(),
                },
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
fn content_text(content: &[UserContentBlock]) -> String {
    content
        .iter()
        .map(|part| match part {
            UserContentBlock::Text { text } => text.clone(),
            UserContentBlock::Image { source } | UserContentBlock::Video { source } => {
                match source {
                    MediaSource::Url { url } => url.clone(),
                    MediaSource::Binary { media_type, .. }
                    | MediaSource::Ref { media_type, .. } => media_type.clone(),
                }
            }
            UserContentBlock::Document { source } => match source {
                DocumentSource::Binary {
                    media_type,
                    file_name,
                    ..
                }
                | DocumentSource::Ref {
                    media_type,
                    file_name,
                    ..
                } => format!("{file_name} {media_type}"),
            },
            UserContentBlock::Reference { reference } => reference.clone(),
            UserContentBlock::Attachment(attachment) => {
                format!("{} {}", attachment.name, attachment.path)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The weight of a block's images and documents; videos weigh nothing.
fn media_tokens(block: &Block) -> u64 {
    match block {
        Block::User(user) => user.content.iter().map(content_media_tokens).sum(),
        Block::Steer(steer) => steer.content.iter().map(content_media_tokens).sum(),
        Block::ToolCall(call) => call
            .output
            .iter()
            .map(|part| match part {
                ToolResultContentBlock::Image {
                    source: ToolMediaSource::Binary { data, .. },
                } => image_tokens(data.len()),
                // Media held by reference has no bytes to count here.
                ToolResultContentBlock::Image {
                    source: ToolMediaSource::Ref { .. },
                } => IMAGE_TOKENS,
                ToolResultContentBlock::Text { .. } | ToolResultContentBlock::Video { .. } => 0,
            })
            .sum(),
        _ => 0,
    }
}

fn content_media_tokens(part: &UserContentBlock) -> u64 {
    match part {
        UserContentBlock::Image {
            source: MediaSource::Binary { data, .. },
        } => image_tokens(data.len()),
        UserContentBlock::Image { .. } => IMAGE_TOKENS,
        UserContentBlock::Document {
            source: DocumentSource::Binary { data, .. },
        } => byte_count(data.len()).div_ceil(DOCUMENT_BYTES_PER_TOKEN),
        // A document held by reference has no bytes to count here.
        UserContentBlock::Document {
            source: DocumentSource::Ref { .. },
        } => 0,
        UserContentBlock::Text { .. }
        | UserContentBlock::Video { .. }
        | UserContentBlock::Reference { .. }
        | UserContentBlock::Attachment(_) => 0,
    }
}

fn image_tokens(bytes: usize) -> u64 {
    IMAGE_TOKENS.max(byte_count(bytes).div_ceil(IMAGE_BYTES_PER_TOKEN))
}

fn byte_count(bytes: usize) -> u64 {
    u64::try_from(bytes).expect("a length fits in 64 bits")
}

#[cfg(test)]
mod tests {
    use demi_core::{
        CompactionBoundaryBlock, CompactionMarkerBlock, ResponseBlock, TextBlock, Timestamp,
        TokenUsage, TurnId, UserBlock,
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
    fn text_counts_utf8_bytes_by_four_rounded_up() {
        assert_eq!(text_tokens("hello"), 2);
        assert_eq!(text_tokens("你好"), 2);
        assert_eq!(text_tokens(""), 0);
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
        assert_eq!(context_tokens(&blocks, None), 1_384);
        blocks.push(user("u2", &"y".repeat(4_000)));
        assert_eq!(context_tokens(&blocks, None), 1_384 + 1_000);
        // A usage above the window is a provider reporting something else.
        blocks.push(response("r2", 2_000_000));
        let unanchored = context_tokens(&blocks, Some(1_000_000));
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
        assert!(context_tokens(&blocks, None) < 10_000);
    }

    #[test]
    fn images_and_documents_weigh_by_their_bytes() {
        let user = Block::User(UserBlock {
            id: "u".try_into().unwrap(),
            turn_id: TurnId::try_from("turn").unwrap(),
            created_at: Timestamp::UNIX_EPOCH,
            model: test_model(),
            content: vec![
                UserContentBlock::Image {
                    source: MediaSource::Binary {
                        data: vec![0; 3_000_000].into(),
                        media_type: "image/png".into(),
                    },
                },
                UserContentBlock::Image {
                    source: MediaSource::Url {
                        url: "https://example.com/a.png".into(),
                    },
                },
                UserContentBlock::Document {
                    source: DocumentSource::Binary {
                        data: vec![0; 40_000].into(),
                        media_type: "application/pdf".into(),
                        file_name: "doc.pdf".into(),
                    },
                },
            ],
            preamble: None,
        });
        let text = text_tokens("image/png\nhttps://example.com/a.png\ndoc.pdf application/pdf");
        assert_eq!(block_tokens(&user), text + 3_000 + 1_600 + 10_000);
    }
}
