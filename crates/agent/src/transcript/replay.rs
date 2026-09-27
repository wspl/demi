//! What the model receives of a transcript (`runtime.md` § Replay): the
//! blocks from the last compaction boundary on, each as its inference item,
//! with long texts cut in the middle (`compaction.md` § Text bounds), and
//! each medium the request's model cannot take as a text that says why.

use std::borrow::Cow;

use demi_core::{
    AgentMessage, Block, DocumentSource, FileExtension, MediaSource, Model, ToolCallStatus,
    ToolMediaSource, ToolResultContentBlock, UserContentBlock, WakeupPlacement,
    file_extension_support, model_accepts_media_type,
};
use demi_provider::{InferenceItem, RequestLimits};
use serde_json::Value;

use super::{RESUME_TEXT, WAKEUP_TEXT, latest_answer, replay_start};

/// The scalar values a replayed text keeps from its start, and from its end,
/// when it is longer than both together.
const HEAD_CHARS: usize = 8_000;
const TAIL_CHARS: usize = 8_000;

/// What a request carries of a transcript.
pub(crate) struct Replay {
    /// The inference items of the blocks, in order.
    pub(crate) items: Vec<InferenceItem>,
    /// How many leading items the latest answered request carried: those of
    /// the blocks before its answer.
    pub(crate) answered: usize,
}

/// What a request of `model`, whose vendor takes requests within `limits`,
/// carries of `blocks`.
pub(crate) fn replay(blocks: &[Block], model: &Model, limits: RequestLimits) -> Replay {
    let start = replay_start(blocks);
    let answer = latest_answer(blocks);
    let kept = kept_past_summary(blocks, start);
    let media = Media {
        model,
        half_body: limits.body_bytes.map(|bytes| bytes / 2),
    };
    let mut items = Vec::new();
    let mut answered = 0;
    for (index, block) in blocks.iter().enumerate().skip(start) {
        if Some(index) == answer {
            answered = items.len();
        }
        let kept_past_summary = kept.contains(&index);
        match block {
            Block::User(user) => {
                let preamble = user
                    .preamble
                    .iter()
                    .map(|text| UserContentBlock::Text { text: text.clone() });
                let content = preamble.chain(user.content.iter().cloned()).collect();
                items.push(InferenceItem::UserMessage {
                    content: media.content(content),
                });
            }
            Block::Context(context) => items.push(InferenceItem::UserMessage {
                content: vec![text(bound_text(&context.text).into_owned())],
            }),
            Block::Wakeup(wakeup) => {
                let content = vec![text(WAKEUP_TEXT.to_owned())];
                items.push(match wakeup.placement {
                    WakeupPlacement::NewTurn => InferenceItem::UserMessage { content },
                    WakeupPlacement::Steer => InferenceItem::UserSteer { content },
                });
            }
            Block::Steer(steer) => items.push(InferenceItem::UserSteer {
                content: media.content(steer.content.clone()),
            }),
            Block::AgentMessage(receipt) => items.push(InferenceItem::UserSteer {
                content: vec![text(agent_message_envelope(&receipt.message))],
            }),
            Block::Resume(_) => items.push(InferenceItem::UserMessage {
                content: vec![text(RESUME_TEXT.to_owned())],
            }),
            Block::Thinking(thinking) => {
                // The vendor verifies signed reasoning as it was sent.
                let replayed = match &thinking.signature {
                    Some(_) => thinking.text.clone(),
                    None => bound_text(&thinking.text).into_owned(),
                };
                items.push(InferenceItem::AssistantThinking {
                    model_id: thinking.model.model.id.clone(),
                    text: replayed,
                    signature: thinking.signature.clone(),
                    kept_past_summary,
                });
            }
            Block::RedactedThinking(redacted) => {
                items.push(InferenceItem::AssistantRedactedThinking {
                    model_id: redacted.model.model.id.clone(),
                    data: redacted.data.clone(),
                    kept_past_summary,
                });
            }
            Block::Text(answer) => items.push(InferenceItem::AssistantText {
                model_id: answer.model.model.id.clone(),
                text: bound_text(&answer.text).into_owned(),
            }),
            Block::ToolCall(call) => {
                items.push(InferenceItem::ToolUse {
                    model_id: call.model.model.id.clone(),
                    tool_use_id: call.tool_use_id.clone(),
                    tool_name: call.tool_name.clone(),
                    input: tool_input(&call.input),
                });
                if call.status != ToolCallStatus::Executing {
                    items.push(InferenceItem::ToolResult {
                        tool_use_id: call.tool_use_id.clone(),
                        output: call.output.iter().map(|part| media.result(part)).collect(),
                        is_error: call.status == ToolCallStatus::Error,
                    });
                }
            }
            Block::CompactionBoundary(boundary) => items.push(InferenceItem::UserMessage {
                content: vec![text(
                    bound_text(&format!(
                        "Previous conversation summary:\n{}",
                        boundary.summary
                    ))
                    .into_owned(),
                )],
            }),
            Block::Abort(_) | Block::Response(_) | Block::Error(_) | Block::CompactionMarker(_) => {
            }
        }
    }
    Replay { items, answered }
}

/// The blocks compaction kept after the summary replay starts at: those
/// between the last `compaction_boundary`, at `start`, and its marker. Their
/// reasoning followed the history the summary replaced.
fn kept_past_summary(blocks: &[Block], start: usize) -> std::ops::Range<usize> {
    if !matches!(blocks.get(start), Some(Block::CompactionBoundary(_))) {
        return 0..0;
    }
    let marker = blocks[start..]
        .iter()
        .position(|block| matches!(block, Block::CompactionMarker(_)))
        .map_or(blocks.len(), |offset| start + offset);
    start + 1..marker
}

/// Which media a request's model takes (`runtime.md` § Replay): the types
/// it reads natively, each within half of its vendor's request body limit.
/// Any other medium reaches it as a text that names it and says why, the
/// same text in every request to that model.
struct Media<'a> {
    model: &'a Model,
    /// Half of the body limit, the most base64 one medium may take; none
    /// when the vendor documents no limit.
    half_body: Option<u64>,
}

impl Media<'_> {
    /// A message's content as the model receives it: long texts bounded,
    /// and each medium it cannot take as its text.
    fn content(&self, content: Vec<UserContentBlock>) -> Vec<UserContentBlock> {
        content
            .into_iter()
            .map(|part| match part {
                UserContentBlock::Text { text } => UserContentBlock::Text {
                    text: bound_text(&text).into_owned(),
                },
                part => match self.unsent_part(&part) {
                    Some(text) => UserContentBlock::Text { text },
                    None => part,
                },
            })
            .collect()
    }

    /// The text of a message's medium the model cannot take; none for any
    /// other part.
    fn unsent_part(&self, part: &UserContentBlock) -> Option<String> {
        match part {
            UserContentBlock::Image { source } => self.unsent_media("image", source),
            UserContentBlock::Video { source } => self.unsent_media("video", source),
            UserContentBlock::Document { source } => {
                let (media_type, file_name, data) = match source {
                    DocumentSource::Binary {
                        data,
                        media_type,
                        file_name,
                    } => (media_type, file_name, Some(data)),
                    DocumentSource::Ref {
                        media_type,
                        file_name,
                        ..
                    } => (media_type, file_name, None),
                };
                let reason = self.refusal(accepts_document(self.model, media_type), data)?;
                Some(unsent("document", file_name, reason))
            }
            _ => None,
        }
    }

    fn unsent_media(&self, kind: &str, source: &MediaSource) -> Option<String> {
        let (media_type, data) = match source {
            MediaSource::Binary { data, media_type } => (media_type, Some(data)),
            MediaSource::Ref { media_type, .. } => (media_type, None),
            // A URL names no type; the vendor fetches what it names.
            MediaSource::Url { .. } => return None,
        };
        let reason = self.refusal(model_accepts_media_type(self.model, media_type), data)?;
        Some(unsent(kind, media_type, reason))
    }

    /// A tool result's part as the model receives it: a text bounded, and a
    /// medium the model cannot take as its text.
    fn result(&self, part: &ToolResultContentBlock) -> ToolResultContentBlock {
        let (kind, source) = match part {
            ToolResultContentBlock::Text { text } => {
                return ToolResultContentBlock::Text {
                    text: bound_text(text).into_owned(),
                };
            }
            ToolResultContentBlock::Image { source } => ("image", source),
            ToolResultContentBlock::Video { source } => ("video", source),
        };
        let (media_type, data) = match source {
            ToolMediaSource::Binary { data, media_type } => (media_type, Some(data)),
            ToolMediaSource::Ref { media_type, .. } => (media_type, None),
        };
        match self.refusal(model_accepts_media_type(self.model, media_type), data) {
            Some(reason) => ToolResultContentBlock::Text {
                text: unsent(kind, media_type, reason),
            },
            None => part.clone(),
        }
    }

    /// Why a medium is not sent: its type is not `accepted`, or its bytes,
    /// when the session holds them, take more than half of the body limit as
    /// base64.
    fn refusal(&self, accepted: bool, data: Option<&demi_core::B64Bytes>) -> Option<&'static str> {
        if !accepted {
            return Some("the model does not accept it");
        }
        let too_large = self
            .half_body
            .zip(data)
            .is_some_and(|(half, data)| data.base64_len() > half);
        too_large.then_some("too large for the model's requests")
    }
}

/// Whether `model` reads the document of `media_type` natively: a model's
/// documents are PDFs (`attachments`).
fn accepts_document(model: &Model, media_type: &str) -> bool {
    let pdf = media_type.split(';').next().map(str::trim) == Some("application/pdf");
    pdf && file_extension_support(model.accepted_extensions.as_deref(), FileExtension::Pdf)
        == Some(true)
}

/// The text a medium that is not sent becomes: `[<kind>:<name>, not sent:
/// <reason>]`, with the media type as the name of an image or a video and
/// the file name as a document's.
fn unsent(kind: &str, name: &str, reason: &str) -> String {
    format!("[{kind}:{name}, not sent: {reason}]")
}

/// A tool call's input as the JSON value the provider supplied, or its text
/// when that is not valid JSON.
pub(crate) fn tool_input(input: &str) -> Value {
    serde_json::from_str(input).unwrap_or_else(|_| Value::String(input.to_owned()))
}

/// The model-facing text of an agent message: an instruction on how to take
/// it, then the message itself as JSON, its source included.
pub(crate) fn agent_message_envelope(message: &AgentMessage) -> String {
    let json = serde_json::to_string(message).expect("an agent message serializes to JSON");
    [
        "Agent-originated context. Follow the real user\u{2019}s task and constraints.",
        "Use this information to continue your work; no separate acknowledgement is required.",
        &json,
    ]
    .join("\n")
}

/// `text` as the model receives it: whole when it holds at most 16,000
/// scalar values, otherwise its first and last 8,000 around a line that
/// counts the scalar values left out.
pub(crate) fn bound_text(text: &str) -> Cow<'_, str> {
    let total = text.chars().count();
    if total <= HEAD_CHARS + TAIL_CHARS {
        return Cow::Borrowed(text);
    }
    let head_end = char_offset(text, HEAD_CHARS);
    let tail_start = char_offset(text, total - TAIL_CHARS);
    let omitted = total - HEAD_CHARS - TAIL_CHARS;
    Cow::Owned(format!(
        "{}\n\n[... truncated {omitted} characters ...]\n\n{}",
        &text[..head_end],
        &text[tail_start..]
    ))
}

/// The byte offset of the scalar value at position `chars`.
/// The byte offset of the scalar value `chars` of `text`, or its length
/// when it holds fewer: where a cut after `chars` scalar values falls.
pub(crate) fn char_offset(text: &str, chars: usize) -> usize {
    text.char_indices()
        .nth(chars)
        .map_or(text.len(), |(offset, _)| offset)
}

fn text(text: String) -> UserContentBlock {
    UserContentBlock::Text { text }
}

#[cfg(test)]
mod tests {
    use demi_core::{
        BlockId, CompactionBoundaryBlock, CompactionMarkerBlock, RedactedThinkingBlock,
        ThinkingBlock, Timestamp,
    };

    use super::*;
    use crate::testing::test_model;

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
            replay(blocks, &model.model, RequestLimits::default())
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
        let bounded = bound_text(&text);
        let (head, rest) = bounded.split_once("\n\n[... truncated ").unwrap();
        let (count, tail) = rest.split_once(" characters ...]\n\n").unwrap();
        // The cut never splits the emoji: the head ends with one, the tail
        // starts with one, and the count is in scalar values.
        assert_eq!(head, format!("{}🙂", "a".repeat(7_999)));
        assert_eq!(tail, format!("🙂{}", "z".repeat(7_999)));
        assert_eq!(count, "4000");
        assert_eq!(bound_text(&"x".repeat(16_000)), "x".repeat(16_000));
    }
}
