//! ByteDance's entries (`crates/backend/src/families.rs`).
//!
//! Scenarios:
//!
//! - BytePlus ModelArk and Doubao Speech are offered from the media catalog
//!   with the endpoints it names, and Volcengine Ark from models.dev, each
//!   with the shapes its vendor makes.
//! - A speech key is checked by Test Connection through a request the service
//!   refuses for its body.
//! - A model asked for a kind it does not make, and speech without a speaker,
//!   are refused before anything is sent.
//! - The catalog's speakers are listed.
//! - Seed Audio's effect and speech, and a Volcengine Ark image, are made
//!   through their vendors.
//!
//! No test calls a real vendor.

use std::sync::Arc;

use base64::{Engine as _, engine::general_purpose::STANDARD};
use demi_provider_common::testing::{MockResponse, MockVendor};
use demi_shared_types::{AudioShape, ImageShape, VideoShape};
use demi_web_api_protocol::providers::{ProviderAnswer, TestResult};
use reqwest::StatusCode;
use serde_json::json;

use crate::agent_profiles::{assert_shows, last_root};
use crate::conversations::{FIRST, Socket};
use crate::subagents::{Scripts, say, shell, tree_among};
use crate::support::Harness;

/// Two seconds of a tone, the plugin's fixture.
const SPEECH: &[u8] = include_bytes!("../../../plugin-audio/tests/plugin-audio/fixtures/speech.mp3");

fn json_answer(status: u16, body: serde_json::Value) -> MockResponse {
    MockResponse::status(status).header("content-type", "application/json").chunk(body.to_string())
}

// About a second: one scripted server and a real device's shell job.
//
// Planted defects:
//
// - A ByteDance vendor of media alone not offered, or offered without the
//   catalog's endpoint.
// - Volcengine Ark's key making no images or videos, or its images not
//   Seedream's.
// - A speech key checked by a listing, a body refusal failing the test or a
//   key refusal passing it.
// - Seed speech's model offered for effects.
// - Speech sent without a speaker.
// - Seed Audio's speech not sent to Seed Audio, or its speaker outside its
//   `references`.
// - The speakers not the catalog's, or Seed Audio listing none.
// - A speed past 2× sent as asked or without its warning.
// - A refusal's words without its code and the request's log ID.
#[tokio::test]
async fn bytedance_entries_make_their_vendors_media_and_check_a_key_by_a_refused_body() {
    let vendor = MockVendor::start().await;
    let catalog = json!({
        "updated": "2026-10-01T00:00:00Z",
        "vendors": [
            { "id": "byteplus", "name": "BytePlus ModelArk", "endpoint": vendor.url("/ark/api/v3") },
            { "id": "doubao-speech", "name": "Doubao Speech", "endpoint": vendor.url("/") }
        ],
        "models": [
            { "vendor": "volcengine", "id": "doubao-seedream-5-0-pro-260628", "kind": "image", "name": "Doubao Seedream 5.0 Pro",
              "image": { "sizes": ["2048x2048", "2816x1584"] } },
            { "vendor": "doubao-speech", "id": "seed-audio-1.0", "kind": "audio", "name": "Audio 1.0",
              "audio": { "makes": ["speech", "effect"] } },
            { "vendor": "doubao-speech", "id": "seed-tts-2.0", "kind": "audio", "name": "TTS 2.0",
              "audio": { "makes": ["speech"], "voices": ["zh_female_xiaohe_uranus_bigtts", "en_male_tim_uranus_bigtts"] } }
        ]
    });
    for _ in 0..2 {
        vendor.respond_at("/media.json", json_answer(200, catalog.clone()));
    }
    let models_dev = json!({ "volcengine": { "id": "volcengine", "name": "Volcengine Ark", "npm": "@ai-sdk/openai-compatible",
                                             "api": vendor.url("/volc/api/v3"), "models": {} } });
    for _ in 0..4 {
        vendor.respond_at("/api.json", json_answer(200, models_dev.clone()));
    }
    // The first key the test checks is one the service does not know; the
    // next it knows, and it refuses the probe's empty prompt.
    let create = "/api/v3/tts/create";
    vendor.respond_at(create, json_answer(401, json!({ "code": 45000010, "message": "invalid api key" })));
    vendor.respond_at(create, json_answer(400, json!({ "code": 40000001, "message": "text_prompt is empty" })));
    for _ in 0..3 {
        vendor.respond_at(create, json_answer(200, json!({ "audio": STANDARD.encode(SPEECH), "duration": 2.0 })));
    }
    vendor.respond_at(
        create,
        json_answer(200, json!({ "code": 40000001, "message": "text_prompt is invalid" })).header("x-tt-logid", "20261018120000AB"),
    );
    let made = demi_agent_store::testing::png(16, 9, 3).into_bytes();
    vendor.respond_at("/volc/api/v3/images/generations", json_answer(200, json!({ "data": [{ "b64_json": STANDARD.encode(&made) }] })));
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _device, root, _tree) = tree_among(
        &scripts,
        Harness::new().with_media_catalog(vendor.url("/media.json")).with_models_dev(vendor.url("/api.json")),
        demi_backend::families::builtin(),
    )
    .await;
    backend.refresh_media_catalog().await;

    let add = |vendor_id: &str, label: &str, key: &str| {
        backend.post(
            "/api/providers",
            Some(&master),
            json!({ "source": "vendor", "vendorId": vendor_id, "label": label, "apiKey": key }),
        )
    };
    let created = add("byteplus", "BytePlus ModelArk", "ark-key").await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    let modelark = created.json::<ProviderAnswer>().provider;
    assert_eq!(serde_json::to_value(&modelark.endpoint).unwrap(), json!(vendor.url("/ark/api/v3")));
    let shapes = |entry: &demi_web_api_protocol::providers::ProviderDto| (entry.media.image.shape, entry.media.video.shape, entry.media.audio.shape);
    assert_eq!(shapes(&modelark), (Some(ImageShape::Seedream), Some(VideoShape::ArkTasks), None));
    let created = add("volcengine", "Volcengine Ark", "volc-key").await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    let volcengine = created.json::<ProviderAnswer>().provider;
    assert_eq!(shapes(&volcengine), (Some(ImageShape::Seedream), Some(VideoShape::ArkTasks), None));
    let created = add("doubao-speech", "Doubao Speech", "wrong-key").await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    let doubao = created.json::<ProviderAnswer>().provider;
    assert_eq!(shapes(&doubao), (None, None, Some(AudioShape::Seed)));

    // Test Connection needs only the key: a refused key fails in the
    // service's words, and a known one passes on the refusal of the
    // probe's body, naming the first of the catalog's models.
    let tested = format!("/api/providers/{}/test", doubao.id);
    let refused = backend.post(&tested, Some(&master), json!({})).await;
    assert_eq!(
        refused.json::<TestResult>(),
        TestResult::Failed { message: "invalid api key".into(), model: None, not_exposed: None }
    );
    let rekeyed = backend.patch(&format!("/api/providers/{}", doubao.id), &master, json!({ "apiKey": "speech-key" })).await;
    assert_eq!(rekeyed.status, StatusCode::OK, "{}", String::from_utf8_lossy(&rekeyed.body));
    let passed = backend.post(&tested, Some(&master), json!({})).await;
    assert_eq!(passed.json::<TestResult>(), TestResult::Passed { model: "Audio 1.0".into() });

    std::fs::write(format!("{root}/expected.mp3"), SPEECH).unwrap();
    std::fs::write(format!("{root}/expected.png"), &made).unwrap();
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    scripts.root(
        FIRST,
        vec![
            shell(
                "t1",
                &[
                    "demi audio effect 'Hi' --model seed-tts-2.0; echo exit=$?",
                    "demi audio speech 'Hi' --model seed-tts-2.0; echo exit=$?",
                    "demi audio voices --model seed-tts-2.0",
                    "echo \"seed-audio speakers: $(demi audio voices --model seed-audio-1.0 | grep -c _uranus_bigtts)\"",
                    "demi audio effect 'a door creaks' --model seed-audio-1.0 > door.mp3 && cmp door.mp3 expected.mp3 && echo same",
                    "demi audio speech 'Stay close' --model seed-audio-1.0 --voice zh_female_xiaohe_uranus_bigtts > hi.mp3 && cmp hi.mp3 expected.mp3 && echo same",
                    "demi image generate 'a pier' --model doubao-seedream-5-0-pro-260628 --aspect 16:9 > pier.png && cmp pier.png expected.png && echo same",
                    "demi audio speech 'Run' --model seed-audio-1.0 --voice en_male_tim_uranus_bigtts --speed 3 > run.mp3; echo exit=$?",
                    "demi audio effect 'thunder' --model seed-audio-1.0; echo exit=$?",
                ]
                .join("\n"),
            ),
            say("made"),
        ],
    );
    socket.chat("m1", "Make a door sound and a picture").await;
    let shown = last_root(&scripts);
    assert_shows(&shown, "demi audio effect: \"seed-tts-2.0\" makes no sound effects; it makes speech\nexit=1");
    assert_shows(&shown, "demi audio speech: Doubao Speech needs --voice; demi audio voices --model seed-tts-2.0 lists them\nexit=1");
    assert_shows(&shown, "zh_female_xiaohe_uranus_bigtts");
    assert_shows(&shown, "en_male_tim_uranus_bigtts");
    assert_shows(&shown, "seed-audio speakers: 2");
    assert_shows(&shown, "Seed Audio takes no --speed 3; sent 2, the nearest it takes");
    assert_shows(
        &shown,
        "demi audio effect: Doubao Speech refused the request: text_prompt is invalid (code 40000001, trace 20261018120000AB)\nexit=1",
    );
    assert_shows(&shown, "0:02 MP3 sound effect from seed-audio-1.0 (Doubao Speech)\nsame");
    assert_shows(&shown, "0:02 MP3 speech from seed-audio-1.0 (Doubao Speech), voice zh_female_xiaohe_uranus_bigtts\nsame");
    assert_shows(&shown, "16×9 PNG from doubao-seedream-5-0-pro-260628 (Volcengine Ark)\nsame");

    let sent = vendor.requests();
    let to = |path: &str| sent.iter().filter(|request| request.uri.path() == path).collect::<Vec<_>>();
    assert!(to("/api/v3/tts/unidirectional").is_empty(), "speech without a speaker is never sent");
    let created = to(create);
    assert_eq!(created.len(), 6);
    assert_eq!(created[2].json(), json!({ "model": "seed-audio-1.0", "text_prompt": "a door creaks", "audio_config": { "format": "mp3" } }));
    assert_eq!(created[3].json()["references"], json!([{ "speaker": "zh_female_xiaohe_uranus_bigtts" }]));
    assert_eq!(created[4].json()["audio_config"], json!({ "format": "mp3", "speech_rate": 100 }));
    let pictured = to("/volc/api/v3/images/generations");
    assert_eq!((pictured[0].header("authorization"), pictured[0].json()["size"].clone()), (Some("Bearer volc-key"), json!("2816x1584")));
    backend.close().await;
}
