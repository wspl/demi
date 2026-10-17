//! An ElevenLabs key's speech, music and sound effects (`audio.md`
//! § Vendors), against a scripted vendor built from ElevenLabs' API
//! reference: Create speech (elevenlabs.io/docs/api-reference/text-to-speech/convert),
//! Compose music (…/music/compose), Create sound effect
//! (…/text-to-sound-effects/convert), List voices (…/voices/search) and
//! List models (…/models/list). No test calls a vendor.

use std::sync::Arc;

use demi_provider_common::{
    Provider, Secret,
    audio::{AudioFormatAsk, AudioOption, AudioRequest, unsent},
    media::{MediaFailure, MediaVendor},
    media_catalog::Catalog,
    testing::{FixedClock, MockResponse, MockVendor},
};
use demi_provider_elevenlabs::{ElevenLabsConfig, ElevenLabsProvider};
use demi_shared_types::audio::Pcm;
use demi_shared_types::{AudioKind, AudioShape};
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

fn request(kind: AudioKind, model: &str, text: &str) -> AudioRequest {
    AudioRequest {
        kind,
        model: model.into(),
        text: text.into(),
        voice: None,
        instructions: None,
        speed: None,
        duration: None,
        instrumental: false,
        seed: None,
        looped: false,
        format: AudioFormatAsk::Mp3,
    }
}

const CATALOG: &[u8] = br#"{ "updated": "2026-10-01T00:00:00Z", "models": [
    { "vendor": "elevenlabs", "id": "music_v1", "kind": "audio", "name": "Eleven Music", "audio": { "makes": ["music"] } },
    { "vendor": "elevenlabs", "id": "eleven_text_to_sound_v2", "kind": "audio", "name": "Sound Effects v2", "audio": { "makes": ["effect"] } },
    { "vendor": "elevenlabs", "id": "eleven_v3", "kind": "audio", "name": "Eleven v3", "audio": { "makes": ["speech"] } }
] }"#;

/// Planted defects this catches: the voice outside the path; the speed
/// outside `voice_settings`; an `output_format` sent for MP3, or WAV of
/// music asked as `wav_44100` (which music does not make) or not marked as
/// 44.1 kHz PCM; the length in seconds where music takes milliseconds;
/// the loop flag left out; the key sent other than in `xi-api-key`.
// One scripted vendor, four requests: milliseconds.
#[tokio::test]
async fn elevenlabs_speaks_composes_and_makes_effects_in_the_format_asked() {
    let vendor = MockVendor::start().await;
    vendor.respond_at("/v1/text-to-speech/JBFqnCBsd6RMkjVDRZzb", MockResponse::status(200).chunk("ID3-speech"));
    vendor.respond_at("/v1/text-to-speech/JBFqnCBsd6RMkjVDRZzb", MockResponse::status(200).chunk("RIFF-speech"));
    vendor.respond_at("/v1/music", MockResponse::status(200).chunk(vec![0u8; 8]));
    vendor.respond_at("/v1/sound-generation", MockResponse::status(200).chunk("OggS"));
    let provider = provider(&vendor);
    let maker = provider.audio().expect("an ElevenLabs key makes audio");
    assert_eq!((maker.vendor(), maker.shape(), maker.needs_voice()), (Some(MediaVendor::Elevenlabs), AudioShape::Elevenlabs, true));
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());
    let catalog = Catalog::decode(CATALOG).unwrap();

    let speech = AudioRequest {
        voice: Some("JBFqnCBsd6RMkjVDRZzb".into()),
        speed: Some(1.1),
        instructions: Some("Whisper".into()),
        ..request(AudioKind::Speech, "eleven_v3", "Hello there.")
    };
    assert_eq!(unsent(maker.shape(), &speech), [AudioOption::Instructions]);
    let mp3 = maker.make(&http, &catalog, speech.clone()).await.result.unwrap();
    assert_eq!((mp3.bytes.as_ref(), mp3.pcm), (b"ID3-speech".as_slice(), None));
    let wav = maker.make(&http, &catalog, AudioRequest { format: AudioFormatAsk::Wav, ..speech }).await.result.unwrap();
    assert_eq!(wav.pcm, None, "speech's WAV is a file");
    let music = AudioRequest {
        duration: Some(15.0),
        instrumental: true,
        seed: Some(9),
        format: AudioFormatAsk::Wav,
        ..request(AudioKind::Music, "music_v1", "Lo-fi beat")
    };
    assert_eq!(unsent(maker.shape(), &music), [AudioOption::Seed]);
    let pcm = maker.make(&http, &catalog, music).await.result.unwrap();
    assert_eq!(pcm.pcm, Some(Pcm::new(44_100, 1, 16).unwrap()));
    let effect = AudioRequest {
        duration: Some(2.5),
        looped: true,
        format: AudioFormatAsk::Opus,
        ..request(AudioKind::Effect, "eleven_text_to_sound_v2", "Door creak")
    };
    maker.make(&http, &catalog, effect).await.result.unwrap();

    let sent = vendor.requests();
    assert_eq!((sent[0].header("xi-api-key"), sent[0].uri.query()), (Some("xi-key"), None));
    assert_eq!(sent[0].json(), json!({ "text": "Hello there.", "model_id": "eleven_v3", "voice_settings": { "speed": 1.1 } }));
    assert_eq!(sent[1].uri.query(), Some("output_format=wav_44100"));
    assert_eq!((sent[2].uri.path(), sent[2].uri.query()), ("/v1/music", Some("output_format=pcm_44100")));
    assert_eq!(sent[2].json(), json!({ "prompt": "Lo-fi beat", "music_length_ms": 15000, "model_id": "music_v1", "force_instrumental": true }));
    assert_eq!(sent[3].uri.query(), Some("output_format=opus_48000_128"));
    assert_eq!(
        sent[3].json(),
        json!({ "text": "Door creak", "duration_seconds": 2.5, "loop": true, "model_id": "eleven_text_to_sound_v2" })
    );
}

/// Planted defects this catches: a model that cannot speak listed as
/// speech; the catalog's music and effect models missing, or listed twice
/// when the listing names one; a refused key read as other than a key
/// failure.
// One scripted vendor, two requests: milliseconds.
#[tokio::test]
async fn the_models_are_the_listed_speech_models_then_the_catalogs_and_a_wrong_key_is_refused() {
    let vendor = MockVendor::start().await;
    vendor.respond_at("/v1/models", json_answer(200, json!([
        { "model_id": "eleven_v3", "name": "Eleven v3", "can_do_text_to_speech": true },
        { "model_id": "eleven_multilingual_sts_v2", "name": "Multilingual STS", "can_do_text_to_speech": false }
    ])));
    vendor.respond_at("/v1/models", json_answer(401, json!({ "detail": { "status": "invalid_api_key", "message": "Invalid API key" } })));
    let provider = provider(&vendor);
    let maker = provider.audio().unwrap();
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());
    let catalog = Catalog::decode(CATALOG).unwrap();

    let models = maker.models(&http, &catalog).await.unwrap();
    let listed: Vec<_> = models.iter().map(|model| (model.id.as_str(), model.kinds.clone())).collect();
    assert_eq!(
        listed,
        [
            ("eleven_v3", vec![AudioKind::Speech]),
            ("music_v1", vec![AudioKind::Music]),
            ("eleven_text_to_sound_v2", vec![AudioKind::Effect]),
        ]
    );
    let refused = maker.models(&http, &catalog).await;
    assert!(matches!(refused, Err(MediaFailure::KeyRefused { .. })), "{refused:?}");
}

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
