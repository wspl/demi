//! The agent's `demi audio`, the `audio` plugin over the port's
//! Providers service: a compatible API whose audio shape and models the
//! agent sets with `demi providers`, its OpenAI Speech request sent to a
//! scripted server and its file written with a ledger row of its
//! seconds; an ElevenLabs entry of media alone, offered from the media
//! catalog, its key checked by Test Connection through its models, the
//! call that takes one of its models sent there, and speech without a
//! voice refused before anything is sent; a model asked for a kind it
//! does not make refused likewise. No test calls a real vendor.

use std::sync::Arc;

use bytes::Bytes;
use demi_provider_common::testing::{MockResponse, MockVendor};
use demi_web_api_protocol::providers::{ProviderAnswer, TestResult};
use reqwest::StatusCode;
use serde_json::json;

use crate::agent_profiles::{assert_shows, last_root};
use crate::conversations::{FIRST, Socket};
use crate::permissions::{decide, permissions, settled};
use crate::subagents::{Scripts, background, say, shell, tree_among};
use crate::support::Harness;
use crate::usage::{Row, rows};

/// Two seconds of a tone, the plugin's fixture.
const SPEECH: &[u8] = include_bytes!("../../../plugin-audio/tests/plugin-audio/fixtures/speech.mp3");

fn json_answer(status: u16, body: serde_json::Value) -> MockResponse {
    MockResponse::status(status).header("content-type", "application/json").chunk(body.to_string())
}

/// The ledger's audio rows.
fn audio_rows(harness: &Harness) -> Vec<Row> {
    rows(harness).into_iter().filter(|row| row.purpose.as_deref() == Some("audio")).collect()
}

// About a second: two scripted servers and a real device's shell jobs of
// two turns, the second woken by the user's decision.
//
// Planted defects: Test Connection of an entry of media alone needing a
// model; a refused key not told in the vendor's words; the catalog's
// longest text left out of `demi audio models <model>`; an audio shape stored for a compatible entry without
// the permission; an audio model added to an entry with no audio shape; a
// compatible server without `/audio/voices` failing the listing; the
// ledger row's seconds left out; ElevenLabs offered without its endpoint,
// or Stability AI offered; an entry of media alone tested with a chat
// message; ElevenLabs speech without a voice sent; a speech model sent a
// music call.
#[tokio::test]
async fn a_compatible_api_and_an_elevenlabs_entry_make_audio_the_agent_asks_for() {
    let proxy_server = MockVendor::start().await;
    proxy_server.respond_at("/v1/audio/voices", MockResponse::status(404).chunk("Not Found"));
    proxy_server.respond_at("/v1/audio/speech", MockResponse::status(200).chunk(Bytes::from_static(SPEECH)));
    let elevenlabs = MockVendor::start().await;
    let listing = json!([{ "model_id": "eleven_v3", "name": "Eleven v3", "can_do_text_to_speech": true }]);
    // The first key the test checks is one ElevenLabs does not know.
    elevenlabs.respond_at("/v1/models", json_answer(401, json!({ "detail": { "status": "invalid_api_key", "message": "Invalid API key" } })));
    for _ in 0..20 {
        elevenlabs.respond_at("/v1/models", json_answer(200, listing.clone()));
    }
    elevenlabs.respond_at("/v1/text-to-speech/george", MockResponse::status(200).chunk(Bytes::from_static(SPEECH)));
    // The media catalog names the ElevenLabs endpoint an entry starts at,
    // which no user changes: the test's catalog names its scripted server.
    let catalog = crate::support::MEDIA_CATALOG
        .replace("https://api.elevenlabs.io", &elevenlabs.url("/"))
        .replace("2026-09-01T00:00:00Z", "2026-10-01T00:00:00Z");
    for _ in 0..2 {
        elevenlabs.respond_at("/media.json", json_answer(200, serde_json::from_str(&catalog).unwrap()));
    }
    let scripts = Arc::new(Scripts::default());
    let (harness, backend, master, _device, root, _tree) = tree_among(
        &scripts,
        Harness::new().with_media_catalog(elevenlabs.url("/media.json")),
        demi_backend::families::builtin(),
    )
    .await;
    backend.refresh_media_catalog().await;

    let created = backend
        .post(
            "/api/providers",
            Some(&master),
            json!({ "source": "compatible", "providerType": "openai", "wireApi": "chat-completions", "label": "Proxy",
                    "baseUrl": proxy_server.url("/v1") }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    let proxy = created.json::<ProviderAnswer>().provider;
    assert_eq!((proxy.media.audio.shape, proxy.media.audio.enabled), (None, true));

    // An ElevenLabs entry starts at the endpoint the media catalog names.
    let created = backend
        .post(
            "/api/providers",
            Some(&master),
            json!({ "source": "vendor", "vendorId": "elevenlabs", "label": "ElevenLabs", "apiKey": "xi-wrong" }),
        )
        .await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    let voices = created.json::<ProviderAnswer>().provider;
    assert_eq!(serde_json::to_value(&voices.endpoint).unwrap(), json!(elevenlabs.url("/")));
    assert_eq!(voices.media.audio.shape, Some(demi_shared_types::AudioShape::Elevenlabs));
    // Test Connection needs only the key: a refused one fails in
    // ElevenLabs' words, and the right one passes, naming a model it lists.
    let tested = format!("/api/providers/{}/test", voices.id);
    let test = || backend.post(&tested, Some(&master), json!({}));
    let listings = || -> Vec<Option<String>> {
        let requests = elevenlabs.requests();
        let listed = requests.iter().filter(|request| request.uri.path() == "/v1/models");
        listed.map(|request| request.header("xi-api-key").map(str::to_owned)).collect()
    };
    assert!(listings().is_empty(), "nothing lists ElevenLabs' models before the test");
    let refused = test().await;
    assert_eq!(refused.status, StatusCode::OK, "{}", String::from_utf8_lossy(&refused.body));
    assert_eq!(
        refused.json::<TestResult>(),
        TestResult::Failed { message: "Invalid API key".into(), model: None, not_exposed: None }
    );
    let rekeyed = backend.patch(&format!("/api/providers/{}", voices.id), &master, json!({ "apiKey": "xi-key" })).await;
    assert_eq!(rekeyed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&rekeyed.body));
    assert_eq!(test().await.json::<TestResult>(), TestResult::Passed { model: "Eleven v3".into() });
    assert_eq!(listings()[..2], [Some("xi-wrong".to_owned()), Some("xi-key".to_owned())]);

    std::fs::write(format!("{root}/expected.mp3"), SPEECH).unwrap();
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    // The first change waits for the permission; once allowed it runs, and
    // a model of a kind the entry does not make is refused.
    scripts.root(
        FIRST,
        vec![
            background(
                "t1",
                "demi providers models add Proxy --id qwen3-tts --name 'Qwen3 TTS' --kind audio; echo exit=$?",
            ),
            say("asked"),
            say("refused"),
            say("refused"),
        ],
    );
    socket.chat("m1", "Set up speech").await;
    assert_shows(
        &last_root(&scripts),
        "waiting for the user's permission to manage providers; its end will report to you, so do not run it again",
    );
    let requested = permissions(&backend, &master, FIRST).await;
    let allowed = decide(&backend, &master, FIRST, requested.requests[0].id().as_str(), "allow").await;
    assert_eq!(allowed.status, StatusCode::NO_CONTENT, "{}", String::from_utf8_lossy(&allowed.body));
    settled(
        &scripts,
        &backend,
        &master,
        FIRST,
        &["The user allowed this conversation to manage providers.", "(t1) ended with exit code 0"],
    )
    .await;
    // A socket of its own, which no frame of the woken turns precedes.
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    scripts.root_anew(
        FIRST,
        vec![
            shell(
                "t2",
                &[
                    "demi providers edit Proxy --audio openai-speech > /dev/null",
                    "demi providers models add Proxy --id qwen3-tts --name 'Qwen3 TTS' --kind audio > /dev/null",
                    "demi providers show Proxy",
                    "demi audio models",
                    "demi audio models eleven_v3 | grep 'Longest text'",
                    "demi audio voices --model qwen3-tts",
                    "demi audio speech 'Hello there' --model qwen3-tts --voice vivian > hi.mp3 && cmp hi.mp3 expected.mp3 && echo same",
                    "demi audio music 'a jingle' --model qwen3-tts; echo exit=$?",
                    "demi audio speech 'Hi' --model eleven_v3; echo exit=$?",
                    "demi audio speech 'Hi' --model eleven_v3 --voice george > el.mp3 && cmp el.mp3 expected.mp3 && echo same",
                ]
                .join("\n"),
            ),
            say("spoken"),
        ],
    );
    socket.chat("m2", "Speak").await;
    let shown = last_root(&scripts);
    assert_shows(
        &shown,
        "demi providers models add: models: qwen3-tts is an audio model, but Proxy makes no audio; choose an audio shape first\nexit=1",
    );
    assert_shows(&shown, "Audio: openai-speech, Audio Generation on\n");
    assert_shows(&shown, "Longest text: 5,000 characters\n");
    assert_shows(&shown, "demi audio voices: note: Proxy lists no voices; --voice is sent as given\n");
    assert_shows(&shown, "0:02 MP3 speech from qwen3-tts (Proxy), voice vivian\nsame");
    assert_shows(&shown, "demi audio music: \"qwen3-tts\" makes no music; it makes speech\nexit=1");
    assert_shows(&shown, "demi audio speech: ElevenLabs needs --voice; demi audio voices --model eleven_v3 lists them\nexit=1");
    assert_shows(&shown, "0:02 MP3 speech from eleven_v3 (ElevenLabs), voice george\nsame");

    let sent = proxy_server.requests();
    let speech = sent.iter().find(|request| request.uri.path() == "/v1/audio/speech").unwrap();
    assert_eq!(
        speech.json(),
        json!({ "model": "qwen3-tts", "input": "Hello there", "voice": "vivian", "response_format": "mp3" })
    );
    assert_eq!(sent.iter().filter(|request| request.uri.path() == "/v1/audio/speech").count(), 1);
    let spoken: Vec<_> = elevenlabs.requests().into_iter().filter(|request| request.uri.path().starts_with("/v1/text-to-speech")).collect();
    assert_eq!(spoken.len(), 1, "speech without a voice is never sent");
    assert_eq!(spoken[0].json(), json!({ "text": "Hi", "model_id": "eleven_v3" }));
    let rows = audio_rows(&harness);
    let seen: Vec<_> = rows
        .iter()
        .map(|row| (row.provider_id.as_str(), row.outcome.as_deref(), row.cost, row.media_seconds))
        .collect();
    assert_eq!(
        seen,
        [(proxy.id.as_str(), Some("answered"), None, Some(2.0)), (voices.id.as_str(), Some("answered"), None, Some(2.0))]
    );
    backend.close().await;
}
