//! What each provider sends of one conversation (`providers.md` § The rule):
//! the session's requests go, in each provider's own format, to a scripted
//! vendor that records them, while the model's answers come from a script.
//! Each request begins with the previous one, byte for byte apart from the
//! Anthropic cache marks, which the vendor does not count as content; a
//! summary request is the latest answered request with the instruction
//! after it; after a summary only the messages change, and Anthropic leaves
//! out the reasoning kept past it; a changed thinking setting changes its
//! own fields alone.

use demi_core::{BlobRef, FileExtension, MediaSource, WireApi};
use demi_provider::{
    InferenceRequest, Provider, ProviderRun, ProviderRuntime, RequestLimits, RuntimeEnv, Secret,
    credentials::{AccountMeta, CredentialPool, MemoryCredentialPool},
    quota::MemorySnapshots,
    testing::{MockResponse, MockVendor, jwt},
};
use demi_provider_anthropic_api::{AnthropicConfig, AnthropicProvider};
use demi_provider_codex::{CodexConfig, CodexProvider, TransportMode};
use demi_provider_google::{GoogleConfig, GoogleProvider};
use demi_provider_grok_build::{GrokConfig, GrokProvider};
use demi_provider_openai_api::{OpenAiConfig, OpenAiProvider, VendorPolicy};
use futures_util::{StreamExt as _, stream};
use serde_json::Value;

use super::*;
use crate::{session::compaction::COMPACTION_SUMMARY_INSTRUCTION, testing::png};

/// A runtime whose runs the session reads from a script, while each request
/// also goes to `real`, a provider's runtime at a scripted vendor, which
/// records it in the provider's own format.
struct Tee {
    script: ScriptedRuntime,
    real: Box<dyn ProviderRuntime>,
}

impl ProviderRuntime for Tee {
    fn run(&mut self, request: InferenceRequest) -> ProviderRun<'_> {
        let Self { script, real } = self;
        let scripted = script.run(request.clone());
        // What the vendor answers does not matter: the request is recorded.
        let sent = real.run(request).collect::<Vec<_>>();
        stream::once(sent)
            .filter_map(|_| async { None })
            .chain(scripted)
            .boxed_local()
    }

    fn fresh(&self) -> Box<dyn ProviderRuntime> {
        Box::new(Self {
            script: self.script.clone(),
            real: self.real.fresh(),
        })
    }

    fn close(&mut self) -> LocalBoxFuture<'_, ()> {
        Box::pin(async {
            self.script.close().await;
            self.real.close().await;
        })
    }

    fn request_limits(&self, model: &demi_core::Model) -> RequestLimits {
        self.real.request_limits(model)
    }
}

/// A provider family's format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    Anthropic,
    Responses,
    ChatCompletions,
    Google,
    Codex,
    GrokBuild,
}

impl Family {
    /// The tag this family's provider puts on the reasoning signatures it
    /// receives, and a signature it replays, as its stream would have made
    /// it.
    fn signature(self, name: &str) -> String {
        let reasoning = || {
            serde_json::json!({ "type": "reasoning", "id": format!("rs_{name}"), "summary": [], "encrypted_content": format!("enc-{name}") })
                .to_string()
        };
        match self {
            Self::Anthropic => format!("anthropic:{name}"),
            Self::Responses | Self::ChatCompletions => format!("openai:{}", reasoning()),
            Self::Google => format!("google:{name}"),
            Self::Codex => format!("codex:{}", reasoning()),
            Self::GrokBuild => format!("grok:{name}"),
        }
    }

    /// The fields that hold the thinking setting, which only a change of the
    /// setting changes.
    fn thinking_fields(self) -> &'static [&'static str] {
        match self {
            Self::Anthropic => &["thinking", "output_config"],
            Self::Responses | Self::Codex => &["reasoning"],
            Self::ChatCompletions | Self::GrokBuild => &["reasoning_effort"],
            Self::Google => &["generationConfig"],
        }
    }

    /// The field that holds the body's messages, whose start a vendor
    /// caches; every other field stays as it was.
    fn sequence(self) -> &'static str {
        match self {
            Self::Anthropic | Self::ChatCompletions | Self::GrokBuild => "messages",
            Self::Responses | Self::Codex => "input",
            Self::Google => "contents",
        }
    }

    /// A runtime of this family's provider at `vendor`.
    async fn runtime(self, vendor: &MockVendor) -> Box<dyn ProviderRuntime> {
        let clock: Arc<dyn demi_core::Clock> = Arc::new(FixedClock(NOW.parse().unwrap()));
        let env = RuntimeEnv {
            http: reqwest::Client::new(),
        };
        let openai = |wire| {
            let config = OpenAiConfig {
                api_key: Secret::try_from("sk-test".to_owned()).unwrap(),
                base_url: Some(vendor.url("/v1").parse().unwrap()),
                wire,
                policy: VendorPolicy::default(),
            };
            OpenAiProvider::new(config, clock.clone())
        };
        let runtime = match self {
            Self::Anthropic => {
                let config = AnthropicConfig {
                    api_key: Secret::try_from("sk-ant-test".to_owned()).unwrap(),
                    base_url: Some(vendor.url("/v1").parse().unwrap()),
                };
                AnthropicProvider::new(config, clock).runtime(env)
            }
            Self::Responses => openai(WireApi::Responses).runtime(env),
            Self::ChatCompletions => openai(WireApi::ChatCompletions).runtime(env),
            Self::Google => {
                let config = GoogleConfig {
                    api_key: Secret::try_from("google-key".to_owned()).unwrap(),
                    base_url: Some(vendor.url("/v1beta").parse().unwrap()),
                };
                GoogleProvider::new(config, clock).runtime(env)
            }
            Self::Codex => {
                let token = jwt(&serde_json::json!({
                    "exp": 1_789_743_600,
                    "https://api.openai.com/auth": { "chatgpt_account_id": "acct-1" }
                }));
                let secret = serde_json::json!({
                    "accessToken": token, "refreshToken": "refresh-1", "idToken": token,
                    "accountId": "acct-1", "lastRefresh": NOW,
                });
                let pool = pool_with("cred-c", secret.to_string()).await;
                let mut config = CodexConfig::new(Some("cred-c".into()));
                config.backend_url = vendor.url("/backend-api").parse().unwrap();
                config.auth_url = vendor.url("").parse().unwrap();
                config.transport = TransportMode::Sse;
                let snapshots = Arc::new(MemorySnapshots::new());
                CodexProvider::new(config, pool, snapshots, reqwest::Client::new(), clock).runtime(env)
            }
            Self::GrokBuild => {
                let secret = serde_json::json!({
                    "accessToken": "session-token", "refreshToken": "refresh-1",
                    "expiresAt": "2030-01-01T00:00:00.000Z", "issuer": vendor.url(""),
                    "clientId": "client-1", "userId": "user-1", "email": "user@example.com",
                });
                let pool = pool_with("cred-g", secret.to_string()).await;
                let mut config = GrokConfig::new(Some("cred-g".into()));
                config.proxy_url = vendor.url("/v1").parse().unwrap();
                config.issuer_url = vendor.url("").parse().unwrap();
                let snapshots = Arc::new(MemorySnapshots::new());
                GrokProvider::new(config, pool, snapshots, reqwest::Client::new(), clock).runtime(env)
            }
        };
        runtime.expect("a provider with a signed-in account builds its runtime")
    }
}

/// When the scripted vendors answer: before the Codex token's expiry.
const NOW: &str = "2026-09-18T14:00:00.000Z";

/// A pool that holds the signed-in account `id` with `secret`.
async fn pool_with(id: &str, secret: String) -> Arc<dyn CredentialPool> {
    let pool = MemoryCredentialPool::new();
    let meta = AccountMeta {
        id: id.into(),
        label: "user@example.com".into(),
        detail: None,
        updated_at: NOW.parse().unwrap(),
        source: "test".into(),
        identity_key: Some("acct-1".into()),
    };
    pool.write(meta, secret).await.unwrap();
    pool.set_active(id).await.unwrap();
    Arc::new(pool)
}

/// What a vendor caches of a body: its messages' parts in order, each with
/// its message's role, since a turn can add parts to the last message; and
/// every other field, which must not change. Cache marks are left out.
fn cached(family: Family, body: &Value) -> (Value, Vec<Value>) {
    let body = unmarked(body);
    let mut fixed = body.clone();
    let sequence = fixed
        .as_object_mut()
        .unwrap()
        .remove(family.sequence())
        .expect("the body holds its messages");
    let mut parts = Vec::new();
    for message in sequence.as_array().unwrap() {
        match family {
            Family::Anthropic => {
                for block in message["content"].as_array().unwrap() {
                    parts.push(serde_json::json!([message["role"], block]));
                }
            }
            Family::Google => {
                for part in message["parts"].as_array().unwrap() {
                    parts.push(serde_json::json!([message["role"], part]));
                }
            }
            _ => parts.push(message.clone()),
        }
    }
    (fixed, parts)
}

/// `value` without Anthropic's cache marks.
fn unmarked(value: &Value) -> Value {
    match value {
        Value::Object(fields) => Value::Object(
            fields
                .iter()
                .filter(|(key, _)| key.as_str() != "cache_control")
                .map(|(key, field)| (key.clone(), unmarked(field)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(unmarked).collect()),
        other => other.clone(),
    }
}

/// Asserts that `later` begins with everything `earlier` sent.
fn extends(family: Family, earlier: &Value, later: &Value, what: &str) {
    let (earlier_fixed, earlier_parts) = cached(family, earlier);
    let (later_fixed, later_parts) = cached(family, later);
    assert_eq!(later_fixed, earlier_fixed, "{family:?}: {what}: the other fields");
    assert!(
        later_parts.len() > earlier_parts.len(),
        "{family:?}: {what}: {later_parts:#?}"
    );
    assert_eq!(
        later_parts[..earlier_parts.len()],
        earlier_parts[..],
        "{family:?}: {what}"
    );
}

/// The scripted conversation, with `family`'s signatures: a message with an
/// image, signed reasoning, two parallel calls of which one returns an image,
/// a steer and a subagent's result during them, a yield after reasoning, a
/// summary, and a message after it. Returns what the vendor received.
async fn conversation(family: Family) -> Vec<Value> {
    let vendor = MockVendor::start().await;
    for _ in 0..8 {
        vendor.respond(MockResponse::event_stream(""));
    }
    let thinking = |name: &str| {
        vec![
            ProviderEvent::ThinkingStart,
            event::thinking(&format!("thinking {name}")),
            ProviderEvent::ThinkingSignature(family.signature(name)),
        ]
    };
    let answer = |text: &str| Turn::Events(vec![event::text(text), event::response(1, 1)]);
    let script = ScriptedRuntime::new([
        Turn::Events(
            [
                thinking("first"),
                vec![
                    event::text("Looking."),
                    event::tool_call("call-look", "look", json!({})),
                    event::tool_call("call-note", "note", json!({ "text": "seen" })),
                    event::response(1, 1),
                ],
            ]
            .concat(),
        ),
        answer("Both seen."),
        Turn::Events(
            [
                thinking("second"),
                vec![
                    event::tool_call("call-yield", "yield", json!({ "durationMs": 600_000 })),
                    event::response(1, 1),
                ],
            ]
            .concat(),
        ),
        answer("The user showed a screenshot and asked twice."),
        answer("After the summary."),
        answer("Thought harder."),
    ]);
    let (look, releases, started) = gated_tool("look");
    let shot = png(3, 2, 2);
    let look = tool("look", {
        let invoke = look.1.clone();
        move |call| {
            let finished = invoke(call);
            let shot = shot.clone();
            Box::pin(async move {
                finished.await?;
                let image = ToolResultContentBlock::Image {
                    source: demi_core::ToolMediaSource::Binary {
                        data: shot,
                        media_type: "image/png".into(),
                    },
                };
                Ok(ToolOutcome {
                    output: vec![image],
                    ..output("")
                })
            })
        }
    });
    let (note, _) = counted("note", "noted");
    let runtime = test_runtime(vec![look, note, yield_tool()]);
    let tee = Tee {
        script: script.clone(),
        real: family.runtime(&vendor).await,
    };
    let store = MemoryTreeStore::new();
    let session = start_with(
        Box::new(tee),
        runtime,
        &store,
        SessionConfig::default(),
        Arc::new(FixedClock(Timestamp::UNIX_EPOCH)),
    )
    .await;
    session
        .update_model(ModelSwitch {
            model: model_reading("stub", "model-a", &[FileExtension::Png]),
            runtime: None,
        })
        .unwrap();
    let photo = png(4, 3, 1);
    let mut held = HeldMedia::default();
    held.hold(BlobRef::of(&photo), photo.clone());
    session.hold_media(held);
    let message = vec![
        UserContentBlock::Text {
            text: "What is on the screen?".into(),
        },
        UserContentBlock::Image {
            source: MediaSource::Ref {
                r#ref: BlobRef::of(&photo),
                media_type: "image/png".into(),
            },
        },
    ];

    let first = session.send(message, turn("t1")).unwrap();
    started.await.unwrap();
    session
        .steer(text("mind the tests"), BlockId::try_from("s1").unwrap())
        .unwrap();
    session
        .accept_agent_message(agent_message("m1"))
        .await
        .unwrap();
    releases.borrow_mut().remove(0).send(()).unwrap();
    first.await.unwrap();
    session
        .send(text("And the rest?"), turn("t2"))
        .unwrap()
        .await
        .unwrap();
    session.compact().unwrap().await.unwrap();
    session
        .send(text("Go on."), turn("t3"))
        .unwrap()
        .await
        .unwrap();
    // The user changes the thinking setting of the same model.
    let mut deeper = model_reading("stub", "model-a", &[FileExtension::Png]);
    deeper.thinking = Some(demi_core::ThinkingConfig::Adaptive {
        effort: "high".into(),
    });
    session
        .update_model(ModelSwitch {
            model: deeper,
            runtime: None,
        })
        .unwrap();
    session
        .send(text("Think harder."), turn("t4"))
        .unwrap()
        .await
        .unwrap();

    assert_eq!(script.remaining(), 0, "{family:?}");
    let requests = vendor.requests();
    assert_eq!(requests.len(), 6, "{family:?}");
    requests.iter().map(|request| request.json()).collect()
}

// Each family's conversation runs its requests over a local server: about
// 0.2 s each.
#[tokio::test(flavor = "local")]
async fn each_providers_requests_begin_with_the_one_before_and_a_summary_or_a_thinking_change_changes_only_what_it_must()
{
    for family in [
        Family::Anthropic,
        Family::Responses,
        Family::ChatCompletions,
        Family::Google,
        Family::Codex,
        Family::GrokBuild,
    ] {
        let bodies = conversation(family).await;
        let [first, second, third, summary, after, deeper] = bodies.as_slice() else {
            unreachable!()
        };
        extends(family, first, second, "the tool results, the steer and the agent message");
        extends(family, second, third, "the next message");
        // The summary request is the latest answered request, the third,
        // with the instruction alone after it.
        extends(family, third, summary, "the summary request");
        let summary_parts = cached(family, summary).1;
        assert_eq!(summary_parts.len(), cached(family, third).1.len() + 1, "{family:?}");
        let instruction = summary_parts.last().unwrap().to_string();
        assert!(
            instruction.contains(COMPACTION_SUMMARY_INSTRUCTION.split('.').next().unwrap()),
            "{family:?}: {instruction}"
        );
        // After the summary the messages start again at it, after the system
        // prompt where it is a message; nothing else changes.
        let (after_fixed, after_parts) = cached(family, after);
        assert_eq!(after_fixed, cached(family, third).0, "{family:?}");
        let system_messages = usize::from(matches!(
            family,
            Family::ChatCompletions | Family::GrokBuild
        ));
        assert_eq!(after_parts[..system_messages], cached(family, third).1[..system_messages]);
        assert!(
            after_parts[system_messages].to_string().contains("Previous conversation summary:"),
            "{family:?}: {after_parts:#?}"
        );
        // The reasoning of the kept answer: Anthropic leaves it out, and the
        // formats that replay reasoning replay it.
        let kept = after.to_string();
        match family {
            Family::Anthropic => assert!(!kept.contains("\"thinking\""), "{kept}"),
            Family::Responses | Family::Codex => assert!(kept.contains("rs_second"), "{kept}"),
            Family::Google => assert!(kept.contains("\"thoughtSignature\":\"second\""), "{kept}"),
            Family::ChatCompletions | Family::GrokBuild => {}
        }
        // A change of the thinking setting changes its fields alone.
        let without_thinking = |body: &Value| {
            let mut body = body.clone();
            for field in family.thinking_fields() {
                body.as_object_mut().unwrap().remove(*field);
            }
            body
        };
        assert_ne!(cached(family, deeper).0, cached(family, after).0, "{family:?}");
        extends(
            family,
            &without_thinking(after),
            &without_thinking(deeper),
            "a changed thinking setting",
        );
    }
}
