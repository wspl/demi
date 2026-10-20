//! MiniMax's entries: a models.dev `minimax` entry makes MiniMax's
//! images, Hailuo's videos and its speech on its endpoint's origin, and a
//! coding plan makes none; a Hailuo job read by the request its model
//! takes; speech without a voice refused before anything is sent, and the
//! voices MiniMax lists. No test calls a real vendor.

use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use demi_provider_common::testing::{MockResponse, MockVendor};
use demi_shared_types::{AudioShape, ImageShape, VideoShape};
use demi_web_api_protocol::providers::ProviderAnswer;
use reqwest::StatusCode;
use serde_json::json;

use crate::agent_profiles::{assert_shows, last_root};
use crate::conversations::{FIRST, Socket};
use crate::subagents::{Scripts, say, shell, tree_among};
use crate::videos::reading_within;

/// A one-second clip, the plugin's fixture.
const CLIP: &[u8] = include_bytes!("../../../plugin-video/tests/plugin-video/fixtures/clip.mp4");

fn json_answer(status: u16, body: serde_json::Value) -> MockResponse {
    MockResponse::status(status).header("content-type", "application/json").chunk(body.to_string())
}

fn ok(mut body: serde_json::Value) -> MockResponse {
    body["base_resp"] = json!({ "status_code": 0, "status_msg": "success" });
    json_answer(200, body)
}

// About a second: one scripted server and a real device's shell job.
//
// Planted defects: a MiniMax key making no media, or a coding plan making
// some; media asked on the text API's path rather than its origin; a v2
// job read at v1's address, as a job read without its model would be;
// the options v2 needs not filled in; speech sent without a voice; the
// voices not MiniMax's list; a usage limit stated on HTTP 200 read as
// made, or its words without its code and the request's trace ID.
#[tokio::test]
async fn a_minimax_key_makes_its_media_on_the_endpoints_origin() {
    let vendor = MockVendor::start().await;
    let catalog = json!({
        "updated": "2026-10-01T00:00:00Z",
        "models": [
            { "vendor": "minimax", "id": "MiniMax-H3", "kind": "video", "name": "MiniMax H3",
              "video": { "durationRange": { "min": 4, "max": 15 }, "resolutions": ["768P", "2K"] }, "api": "v2" },
            { "vendor": "minimax", "id": "image-01", "kind": "image", "name": "MiniMax Image 01" },
            { "vendor": "minimax", "id": "speech-2.8-hd", "kind": "audio", "name": "MiniMax Speech 2.8 HD",
              "audio": { "makes": ["speech"], "voices": "listed" } }
        ]
    });
    for _ in 0..2 {
        vendor.respond_at("/media.json", json_answer(200, catalog.clone()));
    }
    let models_dev = json!({
        "minimax": { "id": "minimax", "name": "MiniMax (minimax.io)", "npm": "@ai-sdk/anthropic",
                     "api": vendor.url("/anthropic/v1"), "models": {} },
        "minimax-coding-plan": { "id": "minimax-coding-plan", "name": "MiniMax Token Plan (minimax.io)", "npm": "@ai-sdk/anthropic",
                                 "api": vendor.url("/anthropic/v1"), "models": {} }
    });
    for _ in 0..4 {
        vendor.respond_at("/api.json", json_answer(200, models_dev.clone()));
    }
    vendor.respond_at("/v2/video_generation", json_answer(200, json!({ "task_id": "424010985738629" })));
    let query = "/v2/query/video_generation/424010985738629";
    vendor.respond_at(query, json_answer(200, json!({ "task": { "status": "running" } })));
    let address = vendor.url("/inference_output/output.mp4");
    vendor.respond_at(query, json_answer(200, json!({ "task": { "status": "succeeded", "content": { "url": address } } })));
    vendor.respond_at("/inference_output/output.mp4", MockResponse::status(200).chunk(Bytes::from_static(CLIP)));
    vendor.respond_at(
        "/v1/get_voice",
        ok(json!({ "system_voice": [{ "voice_id": "English_Graceful_Lady", "voice_name": "Graceful Lady", "description": ["A graceful voice"] }],
                   "voice_cloning": [{ "voice_id": "my-clone", "description": [] }] })),
    );
    vendor.respond_at(
        "/v1/image_generation",
        json_answer(200, json!({ "id": "03ff3cd0820949eb8a410056b5f21d38", "data": null,
                                 "base_resp": { "status_code": 1008, "status_msg": "insufficient balance" } })),
    );
    let scripts = Arc::new(Scripts::default());
    let (_harness, backend, master, _device, root, _tree) = tree_among(
        &scripts,
        reading_within(Duration::from_secs(60)).with_media_catalog(vendor.url("/media.json")).with_models_dev(vendor.url("/api.json")),
        demi_backend::families::builtin(),
    )
    .await;
    backend.refresh_media_catalog().await;

    let add = |vendor_id: &str, label: &str| {
        backend.post(
            "/api/providers",
            Some(&master),
            json!({ "source": "vendor", "vendorId": vendor_id, "label": label, "apiKey": "mm-key" }),
        )
    };
    let shapes = |entry: &demi_web_api_protocol::providers::ProviderDto| (entry.media.image.shape, entry.media.video.shape, entry.media.audio.shape);
    let created = add("minimax", "MiniMax").await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    let minimax = created.json::<ProviderAnswer>().provider;
    assert_eq!(shapes(&minimax), (Some(ImageShape::Minimax), Some(VideoShape::Hailuo), Some(AudioShape::Minimax)));
    let created = add("minimax-coding-plan", "MiniMax Token Plan").await;
    assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
    assert_eq!(shapes(&created.json::<ProviderAnswer>().provider), (None, None, None));

    std::fs::write(format!("{root}/expected.mp4"), CLIP).unwrap();
    let mut socket = Socket::connect(&backend, &master, FIRST).await;
    socket.open().await;
    scripts.root(
        FIRST,
        vec![
            shell(
                "t1",
                &[
                    "demi video generate 'waves on a pier' --model MiniMax-H3 > waves.mp4 && cmp waves.mp4 expected.mp4 && echo same",
                    "demi audio speech 'Hi' --model speech-2.8-hd; echo exit=$?",
                    "demi audio voices --model speech-2.8-hd",
                    "demi image generate 'a pier' --model image-01 > pier.png; echo exit=$?",
                ]
                .join("\n"),
            ),
            say("made"),
        ],
    );
    socket.chat("m1", "Film the pier").await;
    let shown = last_root(&scripts);
    assert_shows(&shown, "demi video generate: asked MiniMax-H3 (MiniMax), job 424010985738629\n");
    assert_shows(&shown, "1 s 64×36 MP4 with sound from MiniMax-H3 (MiniMax)");
    assert_shows(&shown, "same");
    assert_shows(&shown, "demi audio speech: MiniMax needs --voice; demi audio voices --model speech-2.8-hd lists them\nexit=1");
    assert_shows(&shown, "English_Graceful_Lady");
    assert_shows(&shown, "my-clone");
    assert_shows(
        &shown,
        "demi image generate: MiniMax's usage limit is reached (code 1008, trace 03ff3cd0820949eb8a410056b5f21d38)\nexit=1",
    );

    let sent = vendor.requests();
    let to = |path: &str| sent.iter().filter(|request| request.uri.path() == path).collect::<Vec<_>>();
    let asked = to("/v2/video_generation");
    assert_eq!(asked[0].header("authorization"), Some("Bearer mm-key"));
    assert_eq!(
        asked[0].json(),
        json!({ "model": "MiniMax-H3", "content": [{ "type": "text", "text": "waves on a pier" }],
                "resolution": "768P", "duration": 4, "ratio": "16:9" })
    );
    assert_eq!(to(query).len(), 2);
    assert!(to("/v1/t2a_v2").is_empty(), "speech without a voice is never sent");
    backend.close().await;
}
