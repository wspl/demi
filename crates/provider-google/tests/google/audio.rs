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

use demi_provider_common::{
    Provider, Secret,
    audio::{AudioFormatAsk, AudioRequest},
    media::MediaFailure,
    media_catalog::Catalog,
    testing::{FixedClock, MockResponse, MockVendor},
};
use demi_provider_google::{GoogleConfig, GoogleProvider};
use demi_shared_types::audio::Pcm;
use demi_shared_types::AudioKind;
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
