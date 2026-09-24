//! The `generateContent` request body (`models.md` § Request parameters):
//! the transcript as Gemini contents, the tools with their schemas reduced
//! to what Gemini accepts, the output limit and thinking.

use std::{borrow::Cow, collections::HashMap};

use base64::{Engine, engine::general_purpose::STANDARD};
use demi_core::{
    DocumentSource, MediaSource, ThinkingConfig, ToolMediaSource, ToolResultContentBlock,
    UserContentBlock, attachment_tag, is_blank,
};
use demi_provider::{
    InferenceItem, InferenceRequest, ToolDefinition, UnloadedMedia, json_body,
    openai_request::tool_arguments,
};
use serde::Serialize;

use crate::SIGNATURE_TAG;

/// `maxOutputTokens` when the model names no output limit.
const DEFAULT_MAX_OUTPUT_TOKENS: u32 = 32_000;

/// The thinking budget each effort names, in tokens; an effort the ladder
/// does not name thinks like `medium`.
const EFFORT_BUDGETS: [(&str, u32); 5] = [
    ("low", 4_096),
    ("medium", 16_384),
    ("high", 32_768),
    ("xhigh", 65_536),
    ("max", 98_304),
];
const MEDIUM_BUDGET: u32 = 16_384;

/// The JSON body of `request`.
pub(crate) fn encode(request: &InferenceRequest) -> Result<Vec<u8>, UnloadedMedia> {
    let body = Body {
        contents: contents(&request.items)?,
        system_instruction: (!is_blank(&request.system_prompt)).then(|| SystemInstruction {
            parts: [TextPart {
                text: &request.system_prompt,
            }],
        }),
        tools: if request.tools.is_empty() {
            Vec::new()
        } else {
            vec![Tools {
                function_declarations: request
                    .tools
                    .iter()
                    .map(FunctionDeclaration::from)
                    .collect(),
            }]
        },
        generation_config: GenerationConfig {
            max_output_tokens: request
                .output_limit
                .map_or(DEFAULT_MAX_OUTPUT_TOKENS, |limit| limit.get()),
            thinking_config: thinking(request.thinking.as_ref()),
        },
    };
    Ok(json_body(&body))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Body<'a> {
    contents: Vec<Content<'a>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system_instruction: Option<SystemInstruction<'a>>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    tools: Vec<Tools<'a>>,
    generation_config: GenerationConfig,
}

#[derive(Serialize)]
struct SystemInstruction<'a> {
    parts: [TextPart<'a>; 1],
}

#[derive(Serialize)]
struct TextPart<'a> {
    text: &'a str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Tools<'a> {
    function_declarations: Vec<FunctionDeclaration<'a>>,
}

#[derive(Serialize)]
struct FunctionDeclaration<'a> {
    name: &'a str,
    description: &'a str,
    parameters: serde_json::Value,
}

impl<'a> From<&'a ToolDefinition> for FunctionDeclaration<'a> {
    fn from(tool: &'a ToolDefinition) -> Self {
        Self {
            name: &tool.name,
            description: &tool.description,
            parameters: gemini_schema(&serde_json::Value::Object(tool.input_schema.clone())),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GenerationConfig {
    max_output_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking_config: Option<GeminiThinking>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct GeminiThinking {
    include_thoughts: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    thinking_budget: Option<u32>,
}

/// The thinking setting as Gemini's budget: a budget as it is, an effort
/// through its ladder, thinking turned off as a budget of zero without
/// thoughts. Thought summaries are asked for otherwise, because the model
/// thinks, and bills for it, either way.
fn thinking(config: Option<&ThinkingConfig>) -> Option<GeminiThinking> {
    let budget = match config {
        None => None,
        Some(ThinkingConfig::Disabled {}) => {
            return Some(GeminiThinking {
                include_thoughts: false,
                thinking_budget: Some(0),
            });
        }
        Some(ThinkingConfig::Budget { budget_tokens }) => Some(*budget_tokens),
        Some(ThinkingConfig::Adaptive { effort } | ThinkingConfig::Effort { effort, .. }) => {
            let budget = EFFORT_BUDGETS
                .iter()
                .find(|(name, _)| name == effort)
                .map_or(MEDIUM_BUDGET, |(_, budget)| *budget);
            Some(budget)
        }
    };
    Some(GeminiThinking {
        include_thoughts: true,
        thinking_budget: budget,
    })
}

#[derive(Serialize)]
struct Content<'a> {
    role: Role,
    parts: Vec<Part<'a>>,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
enum Role {
    User,
    Model,
}

/// A content part; Gemini tells parts apart by their one field.
#[derive(Serialize)]
#[serde(untagged)]
enum Part<'a> {
    Text {
        text: Cow<'a, str>,
    },
    InlineData {
        #[serde(rename = "inlineData")]
        inline_data: Blob,
    },
    FileData {
        #[serde(rename = "fileData")]
        file_data: FileUri<'a>,
    },
    FunctionCall {
        #[serde(rename = "functionCall")]
        function_call: FunctionCall<'a>,
        #[serde(rename = "thoughtSignature")]
        thought_signature: &'a str,
    },
    FunctionResponse {
        #[serde(rename = "functionResponse")]
        function_response: FunctionResponse<'a>,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Blob {
    mime_type: String,
    data: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct FileUri<'a> {
    file_uri: &'a str,
}

#[derive(Serialize)]
struct FunctionCall<'a> {
    name: &'a str,
    args: Cow<'a, serde_json::Value>,
    id: &'a str,
}

#[derive(Serialize)]
struct FunctionResponse<'a> {
    name: &'a str,
    id: &'a str,
    response: ToolOutput,
}

#[derive(Serialize)]
struct ToolOutput {
    output: String,
}

/// The transcript as Gemini contents; consecutive parts of one role share a
/// content.
///
/// Gemini hands back a thought signature on each function call and requires
/// it verbatim when the call is replayed. The provider streams it as a
/// signed thinking item just before the call, so on replay the thinking item
/// in front of a tool use carries that call's signature. A tool use without
/// a signature of this provider's, typically history from another provider,
/// is replayed as text, and so is its result, because Gemini refuses a
/// function call without a signature.
fn contents(items: &[InferenceItem]) -> Result<Vec<Content<'_>>, UnloadedMedia> {
    let mut contents: Vec<Content<'_>> = Vec::new();
    // A function response names its tool, which only the tool use carries.
    let mut tool_names: HashMap<&str, &str> = HashMap::new();
    let mut as_text: Vec<&str> = Vec::new();
    let mut pending_signature: Option<&str> = None;
    for item in items {
        let (role, parts) = match item {
            InferenceItem::UserMessage { content } | InferenceItem::UserSteer { content } => {
                pending_signature = None;
                (Role::User, user_parts(content)?)
            }
            InferenceItem::AssistantText { text, .. } => {
                pending_signature = None;
                (
                    Role::Model,
                    vec![Part::Text {
                        text: Cow::Borrowed(text),
                    }],
                )
            }
            InferenceItem::AssistantThinking { signature, .. } => {
                // Gemini derives its thought text again; only the signature
                // of the call that follows matters.
                pending_signature = signature.as_deref().and_then(own_signature);
                continue;
            }
            InferenceItem::AssistantRedactedThinking { .. } => continue,
            InferenceItem::ToolUse {
                tool_use_id,
                tool_name,
                input,
                ..
            } => {
                tool_names.insert(tool_use_id, tool_name);
                let part = match pending_signature.take() {
                    Some(thought_signature) => Part::FunctionCall {
                        function_call: FunctionCall {
                            name: tool_name,
                            args: match input {
                                serde_json::Value::Null => Cow::Owned(serde_json::json!({})),
                                input => Cow::Borrowed(input),
                            },
                            id: tool_use_id,
                        },
                        thought_signature,
                    },
                    None => {
                        as_text.push(tool_use_id);
                        let text = format!("[called {tool_name} with {}]", tool_arguments(input));
                        Part::Text {
                            text: Cow::Owned(text),
                        }
                    }
                };
                (Role::Model, vec![part])
            }
            InferenceItem::ToolResult {
                tool_use_id,
                output,
                ..
            } => {
                pending_signature = None;
                let name = tool_names
                    .get(tool_use_id.as_str())
                    .copied()
                    .unwrap_or("tool");
                let parts = if as_text.contains(&tool_use_id.as_str()) {
                    let text = format!("[{name} returned] {}", output_text(output));
                    vec![Part::Text {
                        text: Cow::Owned(text),
                    }]
                } else {
                    function_response(tool_use_id, name, output)?
                };
                (Role::User, parts)
            }
        };
        if parts.is_empty() {
            continue;
        }
        match contents.last_mut() {
            Some(last) if last.role == role => last.parts.extend(parts),
            _ => contents.push(Content { role, parts }),
        }
    }
    Ok(contents)
}

/// A signature this provider received, without its tag.
fn own_signature(tagged: &str) -> Option<&str> {
    tagged
        .strip_prefix(SIGNATURE_TAG)
        .filter(|signature| !signature.is_empty())
}

/// A message's content: text, references and attachment tags as text; images,
/// videos and PDFs as inline data, a video with its audio track; an image or
/// video by URL as file data.
fn user_parts(content: &[UserContentBlock]) -> Result<Vec<Part<'_>>, UnloadedMedia> {
    content
        .iter()
        .map(|block| {
            Ok(match block {
                UserContentBlock::Text { text } => Part::Text {
                    text: Cow::Borrowed(text),
                },
                UserContentBlock::Reference { reference } => Part::Text {
                    text: Cow::Borrowed(reference),
                },
                UserContentBlock::Attachment(attachment) => Part::Text {
                    text: Cow::Owned(attachment_tag(attachment)),
                },
                UserContentBlock::Document { source } => match source {
                    DocumentSource::Binary {
                        data, media_type, ..
                    } => inline(media_type, data),
                    DocumentSource::Ref { r#ref, .. } => {
                        return Err(UnloadedMedia(r#ref.to_string()));
                    }
                },
                UserContentBlock::Image { source } | UserContentBlock::Video { source } => {
                    match source {
                        MediaSource::Binary { data, media_type } => inline(media_type, data),
                        MediaSource::Url { url } => Part::FileData {
                            file_data: FileUri { file_uri: url },
                        },
                        MediaSource::Ref { r#ref, .. } => {
                            return Err(UnloadedMedia(r#ref.to_string()));
                        }
                    }
                }
            })
        })
        .collect()
}

fn inline<'a>(media_type: &str, data: &[u8]) -> Part<'a> {
    Part::InlineData {
        inline_data: Blob {
            mime_type: media_type.to_owned(),
            data: STANDARD.encode(data),
        },
    }
}

/// A tool's result: its text in the function response, which holds JSON
/// only, and the images and videos it returned as inline parts beside it,
/// so the model sees what a command showed.
fn function_response<'a>(
    tool_use_id: &'a str,
    name: &'a str,
    output: &'a [ToolResultContentBlock],
) -> Result<Vec<Part<'a>>, UnloadedMedia> {
    let text = output
        .iter()
        .filter_map(|block| match block {
            ToolResultContentBlock::Text { text } => Some(text.as_str()),
            ToolResultContentBlock::Image { .. } | ToolResultContentBlock::Video { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut parts = vec![Part::FunctionResponse {
        function_response: FunctionResponse {
            name,
            id: tool_use_id,
            response: ToolOutput { output: text },
        },
    }];
    for block in output {
        let source = match block {
            ToolResultContentBlock::Image { source } | ToolResultContentBlock::Video { source } => {
                source
            }
            ToolResultContentBlock::Text { .. } => continue,
        };
        match source {
            ToolMediaSource::Binary { data, media_type } => parts.push(inline(media_type, data)),
            ToolMediaSource::Ref { r#ref, .. } => return Err(UnloadedMedia(r#ref.to_string())),
        }
    }
    Ok(parts)
}

/// A tool's result as text for a call replayed as text: each image or video
/// named by its media type.
fn output_text(output: &[ToolResultContentBlock]) -> String {
    output
        .iter()
        .map(|block| match block {
            ToolResultContentBlock::Text { text } => text.clone(),
            ToolResultContentBlock::Image { source } | ToolResultContentBlock::Video { source } => {
                let (ToolMediaSource::Binary { media_type, .. }
                | ToolMediaSource::Ref { media_type, .. }) = source;
                format!("[{media_type}]")
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The keywords Gemini's function-declaration schema accepts. It is an
/// OpenAPI 3.0 subset rather than JSON Schema, and it refuses the whole
/// request at the first keyword it does not know, such as the
/// `additionalProperties: false` a careful tool author writes.
const GEMINI_SCHEMA_KEYS: [&str; 15] = [
    "type",
    "format",
    "title",
    "description",
    "nullable",
    "enum",
    "items",
    "properties",
    "required",
    "minimum",
    "maximum",
    "minItems",
    "maxItems",
    "anyOf",
    "default",
];

/// A tool's input schema without the keywords Gemini refuses, through the
/// containers that hold nested schemas. The dropped keywords only constrain
/// what the model may send, and the tool checks its own input anyway, so a
/// looser schema is the right failure.
fn gemini_schema(schema: &serde_json::Value) -> serde_json::Value {
    let Some(object) = schema.as_object() else {
        return serde_json::json!({});
    };
    let mut kept = serde_json::Map::new();
    for (key, value) in object {
        if !GEMINI_SCHEMA_KEYS.contains(&key.as_str()) {
            continue;
        }
        let value = match (key.as_str(), value) {
            ("properties", serde_json::Value::Object(properties)) => serde_json::Value::Object(
                properties
                    .iter()
                    .map(|(name, child)| (name.clone(), gemini_schema(child)))
                    .collect(),
            ),
            ("items", child) => gemini_schema(child),
            ("anyOf", serde_json::Value::Array(options)) => {
                serde_json::Value::Array(options.iter().map(gemini_schema).collect())
            }
            (_, value) => value.clone(),
        };
        kept.insert(key.clone(), value);
    }
    serde_json::Value::Object(kept)
}
