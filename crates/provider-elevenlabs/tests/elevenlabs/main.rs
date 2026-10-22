//! An ElevenLabs key's speech, music and sound effects, against a scripted
//! vendor built from ElevenLabs' API reference:
//!
//! - Create speech (elevenlabs.io/docs/api-reference/text-to-speech/convert);
//! - Compose music (…/music/compose);
//! - Create sound effect (…/text-to-sound-effects/convert);
//! - List voices (…/voices/search);
//! - List models (…/models/list).
//!
//! No test calls a vendor.

use std::sync::Arc;

use demi_provider_common::{
    Provider, Secret,
    media_catalog::Catalog,
    testing::{FixedClock, MockResponse, MockVendor},
};
use demi_provider_elevenlabs::{ElevenLabsConfig, ElevenLabsProvider};
use serde_json::{Value, json};

const NOW: &str = "2026-10-17T12:00:00.000Z";

fn provider(vendor: &MockVendor) -> ElevenLabsProvider {
    let config = ElevenLabsConfig {
        api_key: Some(Secret::try_from("xi-key".to_owned()).unwrap()),
        base_url: Some(vendor.url("/").parse().unwrap()),
    };
    ElevenLabsProvider::new(config, Arc::new(FixedClock(NOW.parse().unwrap())))
}

fn json_answer(status: u16, body: Value) -> MockResponse {
    MockResponse::status(status).header("content-type", "application/json").chunk(body.to_string())
}

const CATALOG: &[u8] = br#"{ "updated": "2026-10-01T00:00:00Z", "models": [
    { "vendor": "elevenlabs", "id": "music_v1", "kind": "audio", "name": "Eleven Music", "audio": { "makes": ["music"] } },
    { "vendor": "elevenlabs", "id": "eleven_text_to_sound_v2", "kind": "audio", "name": "Sound Effects v2", "audio": { "makes": ["effect"] } },
    { "vendor": "elevenlabs", "id": "eleven_v3", "kind": "audio", "name": "Eleven v3", "audio": { "makes": ["speech"] } }
] }"#;

/// Planted defects this catches: the listing stopping after its first page,
/// or reading on when `has_more` is false; the account's voices not told
/// from ElevenLabs' own; the languages not read.
// One scripted vendor, two requests: milliseconds.
#[tokio::test]
async fn elevenlabs_lists_its_voices_across_every_page() {
    let vendor = MockVendor::start().await;
    vendor.respond_at("/v2/voices", json_answer(200, json!({ "voices": [
        { "voice_id": "JBFqnCBsd6RMkjVDRZzb", "name": "George", "category": "premade", "description": "Warm",
          "verified_languages": [{ "language": "en" }, { "language": "en" }, { "language": "fr" }] }
    ], "has_more": true, "next_page_token": "n2" })));
    vendor.respond_at("/v2/voices", json_answer(200, json!({ "voices": [
        { "voice_id": "mine", "name": "My Clone", "category": "cloned", "labels": { "language": "ja" } }
    ], "has_more": false, "next_page_token": "n3" })));
    let provider = provider(&vendor);
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());
    let catalog = Catalog::decode(CATALOG).unwrap();
    let voices = provider.audio().unwrap().voices(&http, &catalog, "eleven_v3", None).await.unwrap().unwrap();
    let shown: Vec<_> = voices.iter().map(|voice| (voice.id.as_str(), voice.languages.clone(), voice.own)).collect();
    assert_eq!(
        shown,
        [
            ("JBFqnCBsd6RMkjVDRZzb", vec!["en".to_owned(), "fr".to_owned()], Some(false)),
            ("mine", vec!["ja".to_owned()], Some(true)),
        ]
    );
    let sent = vendor.requests();
    assert_eq!(sent.len(), 2);
    assert_eq!(sent[1].uri.query(), Some("page_size=100&include_total_count=false&next_page_token=n2"));
}
