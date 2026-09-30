//! The conversation title (`product.md` § Conversation titles): from the
//! first message at once, then from the model's answer to a request.

use std::num::NonZeroU32;

use demi_agent::title::{
    INPUT_MAX_CHARS, OUTPUT_CAP, TITLE_INSTRUCTION, TitleError, request_title, title_from_message,
    title_from_response, title_input,
};
use demi_agent_store::testing::test_model;
use demi_core::{ThinkingCapability, ThinkingConfig, ThinkingSummary};
use demi_provider::{
    InferenceItem, PromptCache, ProviderEvent, ProviderFailure, UserPart,
    testing::{ScriptedRuntime, Turn, event},
};
use tokio_util::sync::CancellationToken;

#[test]
fn the_first_message_titles_the_conversation_at_once_on_one_line() {
    assert_eq!(
        title_from_message("  why does\n\tpnpm build   fail  "),
        "why does pnpm build fail"
    );
    let long = "构".repeat(100);
    assert_eq!(title_from_message(&long), "构".repeat(80));
}

#[test]
fn the_input_keeps_the_first_and_the_latest_messages_within_its_bounds() {
    assert_eq!(
        title_input(&["  hello \n world ", "", "second"]),
        "1. hello world\n2. second"
    );
    assert_eq!(title_input::<&str>(&[]), "");
    let long = "字".repeat(500);
    assert_eq!(
        title_input(&[long.as_str()]),
        format!("1. {}", "字".repeat(400))
    );
    // Twenty messages of 400 do not fit in 4,000: the first and the
    // latest that fit stay, and one line stands for the middle.
    let messages: Vec<String> = (1..=20).map(|index| format!("{index:0>400}")).collect();
    let input = title_input(&messages);
    let lines: Vec<&str> = input.lines().collect();
    assert!(lines[0].starts_with("1. "));
    assert_eq!(lines[1], "…");
    assert!(lines.last().unwrap().starts_with("20. "));
    let kept = lines.len() - 2;
    assert!(input.chars().count() <= INPUT_MAX_CHARS + 2);
    assert!(lines[2].starts_with(&format!("{}. ", 21 - kept)));
}

#[test]
fn the_answer_gives_its_first_line_unquoted_within_the_bound() {
    assert_eq!(
        title_from_response("\n  \u{201c}TS2307 after package split\u{201d}  \nmore"),
        Some("TS2307 after package split".to_owned())
    );
    assert_eq!(
        title_from_response("「扫雷游戏」"),
        Some("扫雷游戏".to_owned())
    );
    assert_eq!(title_from_response("\"\"\n"), None);
    assert_eq!(
        title_from_response(&"x".repeat(100)).map(|title| title.len()),
        Some(80)
    );
}

#[tokio::test(flavor = "local")]
async fn a_request_reads_the_instruction_at_the_lowest_effort_and_ignores_thinking() {
    let mut selection = test_model();
    selection.service_tier_id = Some("priority".into());
    selection.model.thinking = vec![
        ThinkingCapability::Budget {
            min_budget_tokens: None,
            max_budget_tokens: None,
            default_budget_tokens: None,
        },
        ThinkingCapability::Effort {
            efforts: vec!["high".into(), "minimal".into(), "medium".into()],
            default_effort: Some("medium".into()),
            summaries: vec![ThinkingSummary::Auto],
            default_summary: None,
        },
    ];
    let script = ScriptedRuntime::new([
        Turn::Events(vec![
            event::thinking("Let me see."),
            event::text("\"Refresh token "),
            event::text("support\"\n"),
            event::response(1, 1),
        ]),
        Turn::Events(vec![ProviderEvent::Error(ProviderFailure {
            message: "quota exhausted".into(),
            code: None,
            diagnostics: None,
            retry_after: None,
        })]),
    ]);
    let mut runtime = script.clone();
    let mut titles = Vec::new();
    // The title's own limit, unless the model's is lower.
    for (messages, model_limit) in [
        (["@src/auth.ts add refresh tokens"], Some(32_000)),
        (["again"], Some(256)),
        (["  "], None),
    ] {
        selection.model.output_limit = model_limit;
        let title = request_title(
            &mut runtime,
            "c1",
            "r1".into(),
            &selection,
            &messages,
            CancellationToken::new(),
        )
        .await;
        titles.push(title);
    }
    let [titled, failed, empty] = <[_; 3]>::try_from(titles).unwrap();

    assert_eq!(titled, Ok(Some("Refresh token support".into())));
    assert_eq!(failed, Err(TitleError::Failed("quota exhausted".into())));
    assert_eq!(empty, Ok(None));
    let requests = script.requests();
    assert_eq!(requests.len(), 2, "text-less messages ask nothing");
    let request = &requests[0];
    assert_eq!(request.system_prompt, TITLE_INSTRUCTION);
    assert_eq!(
        *request.items,
        [InferenceItem::UserMessage {
            content: vec![UserPart::Text("1. @src/auth.ts add refresh tokens".into())]
        }]
    );
    assert!(request.tools.is_empty());
    assert_eq!(
        request.thinking,
        Some(ThinkingConfig::Effort {
            effort: "minimal".into(),
            summary: None
        })
    );
    assert_eq!(request.service_tier_id, None);
    assert_eq!(request.prompt_cache, PromptCache::Off);
    assert_eq!(request.max_output_tokens(), Some(OUTPUT_CAP));
    assert_eq!(request.turn_id, "title:r1");
    assert_eq!(requests[1].max_output_tokens(), NonZeroU32::new(256));
}
