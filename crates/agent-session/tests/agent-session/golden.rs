//! The request after a pass, pinned whole for each provider, the second level
//! of compaction's verification.
//!
//! A short recorded conversation, of a message with an image, signed reasoning,
//! calls with their results, a steer and the user's constraint, is compacted in
//! a 2,000-token window, whose 200 tokens of the user's words keep the first
//! message without its image. The next request each provider's builder makes of
//! it is compared with its golden file in `golden/<builder>.json`.
//!
//! Any change to what the model receives after a pass shows there in review.
//! With `DEMI_GOLDEN_UPDATE=1` the test writes the files instead of comparing
//! them.

use demi_agent_store::testing::png;
use demi_provider_common::InferenceRequest;
use demi_shared_types::{FileExtension, MediaSource};
use serde_json::Value;

use super::generated::{BUILDERS, Builder, is_summary, valid};
use super::*;

/// The tag `builder`'s provider puts on the reasoning signatures it
/// receives, so that its builder replays them.
fn tag(builder: Builder) -> &'static str {
    match builder {
        Builder::Anthropic | Builder::ClaudeCode => "anthropic:",
        Builder::Responses | Builder::ChatCompletions => "openai:",
        Builder::Codex => "codex:",
        Builder::Google => "google:",
        Builder::Grok => "grok:",
    }
}

/// Reasoning signed for `builder`'s vendor.
fn reasoning(builder: Builder, name: &str) -> Vec<ProviderEvent> {
    let signature = match builder {
        Builder::Responses | Builder::ChatCompletions | Builder::Codex => json!({
            "type": "reasoning",
            "id": format!("rs_{name}"),
            "summary": [],
            "encrypted_content": format!("enc-{name}")
        })
        .to_string(),
        _ => format!("sig-{name}"),
    };
    vec![
        ProviderEvent::ThinkingStart,
        event::thinking(&format!("Thinking about {name}.")),
        ProviderEvent::ThinkingSignature(format!("{}{signature}", tag(builder))),
    ]
}

/// The recorded conversation with `builder`'s reasoning, compacted, and
/// the request after the pass.
async fn request_after_a_pass(builder: Builder) -> InferenceRequest {
    let provider = ScriptedRuntime::new([
        Turn::Events(
            [
                reasoning(builder, "the log"),
                vec![
                    event::text("Looking at the build log."),
                    event::tool_call("call-look", "look", json!({ "path": "build.log" })),
                    event::response(1, 1),
                ],
            ]
            .concat(),
        ),
        Turn::Events(vec![
            event::tool_call("call-note", "note", json!({ "text": "missing libssl" })),
            event::response(1, 1),
        ]),
        Turn::Events(vec![
            event::text("The linker misses `libssl`."),
            event::response(1, 1),
        ]),
        Turn::Events(
            [
                reasoning(builder, "the fix"),
                vec![
                    event::tool_call("call-test", "test", json!({ "suite": "release" })),
                    event::response(1, 1),
                ],
            ]
            .concat(),
        ),
        Turn::Events(vec![
            event::text("I added `libssl` to the link line; the tests pass."),
            event::response(1, 1),
        ]),
        Turn::Events(vec![
            event::text("## Goal\n- Fix the release build.\n## Constraints and preferences\n- \"don't touch the migration files\"\n## Progress\n- `libssl` added to the link line; tests pass.\n## Next step\n- Ship it."),
            event::response(1, 1),
        ]),
        Turn::Events(vec![event::text("Shipping."), event::response(1, 1)]),
    ]);
    let (look, releases, started) = gated_tool("look");
    let look = tool("look", {
        let invoke = look.1.clone();
        move |call| {
            let finished = invoke(call);
            Box::pin(async move {
                finished.await?;
                Ok(output("error: undefined symbol SSL_new"))
            })
        }
    });
    let (note, _) = counted("note", "noted");
    let test = tool("test", |_| {
        Box::pin(async { Ok(output(&format!("running 40 tests\n{}test result: ok", "test ... ok\n".repeat(100)))) })
    });
    let store = MemoryTreeStore::new();
    let config = SessionConfig {
        compaction: CompactionConfig { automatic: false },
        ..SessionConfig::default()
    };
    let session = start(&provider, vec![look, note, test], &store, config).await;
    let mut model = model_reading("stub", "model-a", &[FileExtension::Png]);
    model.model.context_window = 2_000;
    session
        .update_model(ModelSwitch {
            compaction_blocked: None,
            model: Box::new(model),
            runtime: None,
        })
        .unwrap();
    let shot = png(3, 2, 7);
    let mut held = HeldMedia::default();
    held.hold(BlobRef::of(shot.as_bytes()), shot.clone());
    session.hold_media(held);
    let message = vec![
        UserContentBlock::Text {
            text: "The release build fails; here is the screen.".into(),
        },
        UserContentBlock::Image {
            source: MediaSource::Ref {
                r#ref: BlobRef::of(shot.as_bytes()),
                media_type: "image/png".into(),
                width: None,
                height: None,
            },
        },
    ];
    let first = session.send(message, turn("t1")).unwrap();
    started.await.unwrap();
    session
        .steer(text("Use the release profile."), BlockId::try_from("s1").unwrap())
        .unwrap();
    releases.borrow_mut().remove(0).send(()).unwrap();
    first.await.unwrap();
    session
        .send(text("Fix it, and don't touch the migration files."), turn("t2"))
        .unwrap()
        .await
        .unwrap();
    session.compact().unwrap().await.unwrap();
    session.send(text("Ship it."), turn("t3")).unwrap().await.unwrap();

    let requests = provider.requests();
    assert_eq!(provider.remaining(), 0, "{builder:?}");
    assert!(is_summary(&requests[5]), "{builder:?}");
    requests.last().unwrap().clone()
}

/// The golden file of `builder`.
fn golden(builder: Builder) -> std::path::PathBuf {
    let name = format!("{builder:?}").to_lowercase();
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/agent-session/golden")
        .join(format!("{name}.json"))
}

/// `value` with each image's base64 as it is, every other value as the
/// builder made it: pretty, one field per line, for review.
fn pretty(value: &Value) -> String {
    serde_json::to_string_pretty(value).expect("JSON serializes") + "\n"
}

// Seven conversations of seven scripted requests: a few milliseconds.
#[tokio::test(flavor = "local")]
async fn the_request_after_a_pass_is_pinned_for_every_provider() {
    let update = std::env::var_os("DEMI_GOLDEN_UPDATE").is_some();
    for builder in BUILDERS {
        let request = request_after_a_pass(builder).await;
        valid(builder, &request).unwrap_or_else(|problem| panic!("{builder:?}: {problem}"));
        let pinned = pretty(&builder.json(&request));
        let path = golden(builder);
        if update {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, &pinned).unwrap();
            continue;
        }
        let expected = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        assert!(
            pinned == expected,
            "{builder:?}: the request after a pass changed; review it and run with DEMI_GOLDEN_UPDATE=1\n{pinned}"
        );
    }
}
