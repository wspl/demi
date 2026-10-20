//! A Gemini API key's speech and Lyria music, against a scripted vendor built
//! from Google's speech and music guides
//! (ai.google.dev/gemini-api/docs/speech-generation, …/music-generation) and
//! the Interactions API reference (ai.google.dev/api/interactions-api).
//!
//! The tests cover:
//!
//! - the interaction with the instructions as the turn's `speech_metadata` and
//!   the voice in `speech_config`;
//! - its `model_output` step's audio as base64, WAV kept and raw `audio/l16`
//!   marked to be wrapped;
//! - the voices, every page.
//!
//! No test calls a vendor.

use std::sync::Arc;

use bytes::Bytes;

use demi_provider_common::{
    Provider, Secret,
    audio::{AudioFormatAsk, AudioOption, AudioRequest, unsent},
    media::{MediaFailure, MediaVendor},
    media_catalog::Catalog,
    testing::{FixedClock, MockResponse, MockVendor},
};
use demi_provider_google::{GoogleConfig, GoogleProvider};
use demi_shared_types::audio::Pcm;
use demi_shared_types::{AudioKind, AudioShape};
use serde_json::{Value, json};

use crate::NOW;

fn provider(vendor: &MockVendor) -> GoogleProvider {
    let config = GoogleConfig {
        api_key: Some(Secret::try_from("google-key".to_owned()).unwrap()),
        base_url: Some(vendor.url("/v1beta").parse().unwrap()),
        makes_images: false,
        makes_videos: false,
        makes_audio: true,
    };
    GoogleProvider::new(config, Arc::new(FixedClock(NOW.parse().unwrap())))
}

fn json_answer(status: u16, body: Value) -> MockResponse {
    MockResponse::status(status).header("content-type", "application/json").chunk(body.to_string())
}

fn request(kind: AudioKind, model: &str) -> AudioRequest {
    AudioRequest {
        kind,
        model: model.into(),
        text: "Say hello.".into(),
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

/// An interaction whose output step holds `data`, base64, of `mime_type`.
fn interaction(mime_type: &str, data: &str) -> MockResponse {
    json_answer(200, json!({ "id": "int-1", "status": "completed", "steps": [
        { "type": "user_input", "content": [{ "type": "text", "text": "Say hello." }] },
        { "type": "model_output", "content": [{ "type": "audio", "mime_type": mime_type, "data": data }] }
    ], "usage": { "total_input_tokens": 5, "total_output_tokens": 40 } }))
}

fn catalog() -> Catalog {
    Catalog::decode(br#"{ "updated": "2026-10-01T00:00:00Z", "models": [] }"#).unwrap()
}

/// Planted defects this catches:
///
/// - the instructions sent as text of the turn rather than its
///   `speech_metadata` style;
/// - the voice outside `speech_config`;
/// - a WAV block marked as PCM, or an `audio/l16` one not marked, or at the
///   wrong rate;
/// - Lyria's prompt sent as a turn, or WAV not asked when `--format wav` does;
/// - a model without TTS or Lyria listed.
// One scripted vendor, four requests: milliseconds.
#[tokio::test]
async fn gemini_speaks_through_an_interaction_and_lyria_composes_with_the_format_asked() {
    let vendor = MockVendor::start().await;
    vendor.respond_at("/v1beta/models", json_answer(200, json!({ "models": [
        { "name": "models/gemini-3.1-flash-tts-preview", "supportedGenerationMethods": ["generateContent"] },
        { "name": "models/lyria-3-clip-preview" },
        { "name": "models/gemini-3-flash", "supportedGenerationMethods": ["generateContent"] }
    ] })));
    vendor.respond_at("/v1beta/interactions", interaction("audio/wav", "UklGRg=="));
    vendor.respond_at("/v1beta/interactions", interaction("audio/L16;codec=pcm;rate=24000", "AAA="));
    vendor.respond_at("/v1beta/interactions", interaction("audio/wav", "UklGRg=="));
    let provider = provider(&vendor);
    let maker = provider.audio().expect("a Gemini key makes audio");
    assert_eq!((maker.vendor(), maker.shape()), (Some(MediaVendor::Google), AudioShape::Gemini));
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());
    let catalog = catalog();

    let models = maker.models(&http, &catalog).await.unwrap();
    let listed: Vec<_> = models.iter().map(|model| (model.id.as_str(), model.kinds.clone())).collect();
    assert_eq!(
        listed,
        [("gemini-3.1-flash-tts-preview", vec![AudioKind::Speech]), ("lyria-3-clip-preview", vec![AudioKind::Music])]
    );
    let spoken = AudioRequest {
        voice: Some("Kore".into()),
        instructions: Some("Say cheerfully".into()),
        speed: Some(1.5),
        ..request(AudioKind::Speech, "gemini-3.1-flash-tts-preview")
    };
    assert_eq!(unsent(maker.shape(), &spoken), [AudioOption::Speed]);
    let wav = maker.make(&http, &catalog, spoken).await.result.unwrap();
    assert_eq!((wav.bytes.as_ref(), wav.pcm, wav.vendor_id.as_deref()), (b"RIFF".as_slice(), None, Some("int-1")));
    let pcm = maker.make(&http, &catalog, request(AudioKind::Speech, "gemini-2.5-flash-preview-tts")).await.result.unwrap();
    assert_eq!(pcm.bytes.len(), 3 - 1, "two bytes of one 16-bit sample decoded from AAA=");
    assert_eq!(pcm.pcm, Some(Pcm::new(24_000, 1, 16).unwrap()));
    let music = AudioRequest {
        text: "A calm piano piece".into(),
        format: AudioFormatAsk::Wav,
        duration: Some(30.0),
        seed: Some(3),
        ..request(AudioKind::Music, "lyria-3-clip-preview")
    };
    assert_eq!(unsent(maker.shape(), &music), [AudioOption::Duration, AudioOption::Seed]);
    maker.make(&http, &catalog, music).await.result.unwrap();

    let sent = vendor.requests();
    assert_eq!(sent[1].header("x-goog-api-key"), Some("google-key"));
    assert_eq!(
        sent[1].json(),
        json!({ "model": "gemini-3.1-flash-tts-preview",
                "input": [{ "type": "user_input", "content": [{ "type": "text", "text": "Say hello.",
                    "annotations": [{ "type": "speech_metadata", "style": "Say cheerfully" }] }] }],
                "response_format": { "type": "audio" },
                "generation_config": { "speech_config": [{ "voice": "Kore" }] } })
    );
    assert_eq!(
        sent[3].json(),
        json!({ "model": "lyria-3-clip-preview", "input": "A calm piano piece",
                "response_format": { "type": "audio", "mime_type": "audio/wav" } })
    );
}

/// An interaction whose output holds no audio fails with what the model
/// said instead.
// One scripted vendor, one request: milliseconds.
#[tokio::test]
async fn an_interaction_without_audio_fails_in_the_models_words() {
    let vendor = MockVendor::start().await;
    vendor.respond_at("/v1beta/interactions", json_answer(200, json!({ "id": "int-2", "steps": [
        { "type": "model_output", "content": [{ "type": "text", "text": "I can't sing that." }] }
    ] })));
    let provider = provider(&vendor);
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());
    let failure = provider.audio().unwrap().make(&http, &catalog(), request(AudioKind::Music, "lyria-3-pro-preview")).await.result;
    assert!(matches!(failure, Err(MediaFailure::Refused { ref message, .. }) if message == "I can't sing that."), "{failure:?}");
}

/// Planted defects this catches: the listing stopping after its first page;
/// the search or the page token not sent.
// One scripted vendor, two requests: milliseconds.
#[tokio::test]
async fn gemini_lists_its_voices_across_every_page() {
    let vendor = MockVendor::start().await;
    vendor.respond_at("/v1beta/voices", json_answer(200, json!({ "voices": [
        { "id": "my-voice", "display_name": "My Voice", "description": "Cloned" }
    ], "next_page_token": "p2" })));
    vendor.respond_at("/v1beta/voices", json_answer(200, json!({ "voices": [
        { "id": "Kore", "display_name": "Kore", "description": "Firm" }
    ] })));
    let provider = provider(&vendor);
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());
    let voices = provider.audio().unwrap().voices(&http, &catalog(), "gemini-3.1-flash-tts-preview", Some("o")).await.unwrap().unwrap();
    assert_eq!(voices.iter().map(|voice| voice.id.as_str()).collect::<Vec<_>>(), ["my-voice", "Kore"]);
    let sent = vendor.requests();
    assert_eq!(sent[0].uri.query(), Some("page_size=1000&search=o"));
    assert_eq!(sent[1].uri.query(), Some("page_size=1000&search=o&page_token=p2"));
}

/// Raw PCM is wrapped only as its media type states it.
///
/// - A type with no rate, or one that does not read, fails the call rather than
///   being taken as 24 kHz.
/// - Stereo stays stereo.
/// - PCM that ends inside a sample fails.
///
/// Planted defects this catches:
///
/// - a missing rate filled in with 24 kHz;
/// - the channels the type states dropped for mono;
/// - a broken last sample written on.
// One scripted vendor, four requests: milliseconds.
#[tokio::test]
async fn pcm_is_taken_only_as_its_type_states_it() {
    let vendor = MockVendor::start().await;
    vendor.respond_at("/v1beta/interactions", interaction("audio/L16;codec=pcm", "AAAA"));
    vendor.respond_at("/v1beta/interactions", interaction("audio/L16;rate=fast", "AAAA"));
    vendor.respond_at("/v1beta/interactions", interaction("audio/L16;rate=48000;channels=2", "AAAAAA=="));
    vendor.respond_at("/v1beta/interactions", interaction("audio/L16;rate=48000;channels=2", "AAAA"));
    let provider = provider(&vendor);
    let maker = provider.audio().unwrap();
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());
    let speak = || request(AudioKind::Speech, "gemini-2.5-flash-preview-tts");
    let cause = |result: Result<_, MediaFailure>| match result {
        Err(MediaFailure::Unanswered { cause, .. }) => cause,
        other => panic!("not refused: {:?}", other.map(|_: demi_provider_common::audio::MadeAudio| ())),
    };

    assert_eq!(
        cause(maker.make(&http, &catalog(), speak()).await.result),
        "its audio's type audio/L16;codec=pcm does not say its sample rate"
    );
    assert_eq!(cause(maker.make(&http, &catalog(), speak()).await.result), "its audio's type audio/L16;rate=fast is not one this reads");
    let stereo = maker.make(&http, &catalog(), speak()).await.result.unwrap();
    assert_eq!((stereo.bytes.len(), stereo.pcm), (4, Some(Pcm::new(48_000, 2, 16).unwrap())));
    assert_eq!(
        cause(maker.make(&http, &catalog(), speak()).await.result),
        "its PCM of 3 bytes does not end on a whole sample of 4 bytes"
    );
}

/// An answer whose base64 audio is over 256 MiB once decoded fails, though its
/// JSON is within what a JSON answer may be.
///
/// Planted defect this catches: the base64 counted as it arrives only, which
/// lets through a third more than a file answer may hold.
// One scripted vendor streaming 342 MiB of base64: a few seconds, and
// about 950 MiB of memory at its peak (the body, its text and the
// decoded audio).
#[tokio::test]
async fn base64_audio_is_counted_as_decoded() {
    let vendor = MockVendor::start().await;
    let quarter = Bytes::from(vec![b'A'; 1024 * 1024]);
    let mut response = MockResponse::status(200)
        .header("content-type", "application/json")
        .chunk(r#"{"steps":[{"type":"model_output","content":[{"type":"audio","mime_type":"audio/wav","data":""#);
    // 342 MiB of base64 is 256.5 MiB of audio.
    for _ in 0..342 {
        response = response.chunk(quarter.clone());
    }
    vendor.respond_at("/v1beta/interactions", response.chunk(r#""}]}]}"#));
    let provider = provider(&vendor);
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());

    let answer = provider.audio().unwrap().make(&http, &catalog(), request(AudioKind::Speech, "gemini-3.1-flash-tts-preview")).await;

    match answer.result {
        Err(MediaFailure::Unanswered { cause, code: None, .. }) => assert_eq!(cause, "its audio is over 256 MiB"),
        other => panic!("not refused for its size: {:?}", other.map(|made| made.bytes.len())),
    }
}
