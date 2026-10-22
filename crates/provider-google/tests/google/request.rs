//! The `generateContent` request a run sends
//! (`crates/shared-types/src/model.rs`,
//! `crates/provider-common/src/endpoint.rs`).

use std::sync::Arc;

use demi_provider_common::{
    InferenceItem, InferenceRequest, ResultPart, ToolDefinition, UserPart,
    testing::inference_request,
};
use demi_shared_types::ThinkingConfig;
use serde_json::{Value, json};

use crate::body_of;

fn text(text: &str) -> Vec<UserPart> {
    vec![UserPart::Text(text.into())]
}

fn request_with(items: Vec<InferenceItem>) -> InferenceRequest {
    let mut request = inference_request();
    request.items = items.into();
    request
}

fn signed(signature: &str) -> InferenceItem {
    InferenceItem::AssistantThinking {
        model_id: "gemini".into(),
        text: String::new(),
        signature: Some(signature.into()),
        kept_past_summary: false,
    }
}

fn tool_use(id: &str, name: &str, input: Value) -> InferenceItem {
    InferenceItem::ToolUse {
        model_id: "gemini".into(),
        tool_use_id: id.into(),
        tool_name: name.into(),
        input,
    }
}

fn tool_result(id: &str, output: Vec<ResultPart>) -> InferenceItem {
    InferenceItem::ToolResult {
        tool_use_id: id.into(),
        output,
        is_error: false,
    }
}

#[tokio::test]
async fn the_system_prompt_tools_and_thinking_budget_land_in_the_body() {
    let tool = ToolDefinition {
        name: "shell_exec".into(),
        description: "run".into(),
        input_schema: json!({ "type": "object" }).as_object().unwrap().clone(),
    };
    let mut request = request_with(vec![InferenceItem::UserMessage {
        content: text("hi"),
    }]);
    request.system_prompt = "you are a shell".into();
    request.tools = Arc::new([tool]);
    request.thinking = Some(ThinkingConfig::Effort {
        effort: "high".into(),
        summary: None,
    });
    let body = body_of(request.clone()).await;
    assert_eq!(
        body["systemInstruction"],
        json!({ "parts": [{ "text": "you are a shell" }] })
    );
    assert_eq!(
        body["tools"],
        json!([{ "functionDeclarations": [{ "name": "shell_exec", "description": "run", "parameters": { "type": "object" } }] }])
    );
    assert_eq!(
        body["contents"],
        json!([{ "role": "user", "parts": [{ "text": "hi" }] }])
    );
    assert_eq!(
        body["generationConfig"]["thinkingConfig"],
        json!({ "includeThoughts": true, "thinkingBudget": 32_768 })
    );

    let cases = [
        (
            Some(ThinkingConfig::Budget {
                budget_tokens: 2_048,
            }),
            json!({ "includeThoughts": true, "thinkingBudget": 2_048 }),
        ),
        (
            Some(ThinkingConfig::Adaptive {
                effort: "unheard-of".into(),
            }),
            json!({ "includeThoughts": true, "thinkingBudget": 16_384 }),
        ),
        (
            Some(ThinkingConfig::Disabled {}),
            json!({ "includeThoughts": false, "thinkingBudget": 0 }),
        ),
        (None, json!({ "includeThoughts": true })),
    ];
    for (thinking, expected) in cases {
        let mut request = request.clone();
        request.thinking = thinking.clone();
        assert_eq!(
            body_of(request).await["generationConfig"]["thinkingConfig"],
            expected,
            "{thinking:?}"
        );
    }
    let blank = body_of(inference_request()).await;
    assert!(blank.get("systemInstruction").is_none() && blank.get("tools").is_none());
}

#[tokio::test]
async fn tool_schemas_are_reduced_to_the_keywords_gemini_accepts() {
    let schema = json!({
        "type": "object",
        "$schema": "https://json-schema.org/draft/2020-12/schema",
        "additionalProperties": false,
        "required": ["script"],
        "properties": {
            "script": { "type": "string", "minLength": 1, "description": "the script" },
            "tags": { "type": "array", "items": { "type": "string", "pattern": "^[a-z]+$" } },
            "mode": { "anyOf": [{ "type": "string", "const": "fast" }, { "type": "null" }] },
        },
    });
    let mut request = inference_request();
    request.tools = Arc::new([ToolDefinition {
        name: "shell_exec".into(),
        description: "run".into(),
        input_schema: schema.as_object().unwrap().clone(),
    }]);
    let body = body_of(request).await;
    assert_eq!(
        body["tools"][0]["functionDeclarations"][0]["parameters"],
        json!({
            "type": "object",
            "required": ["script"],
            "properties": {
                "script": { "type": "string", "description": "the script" },
                "tags": { "type": "array", "items": { "type": "string" } },
                "mode": { "anyOf": [{ "type": "string" }, { "type": "null" }] },
            },
        })
    );
}

#[tokio::test]
async fn a_tool_call_replays_with_the_signature_of_the_thinking_item_in_front_of_it() {
    let body = body_of(request_with(vec![
        InferenceItem::UserMessage {
            content: text("list files"),
        },
        signed("google:sig-abc"),
        tool_use("call-1", "shell_exec", json!({ "command": "ls" })),
        tool_result("call-1", vec![ResultPart::Text("a.md".into())]),
    ]))
    .await;
    assert_eq!(
        body["contents"],
        json!([
            { "role": "user", "parts": [{ "text": "list files" }] },
            { "role": "model", "parts": [{ "functionCall": { "name": "shell_exec", "args": { "command": "ls" }, "id": "call-1" }, "thoughtSignature": "sig-abc" }] },
            { "role": "user", "parts": [{ "functionResponse": { "name": "shell_exec", "id": "call-1", "response": { "output": "a.md" } } }] },
        ])
    );
}
