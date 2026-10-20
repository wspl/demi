//! An OpenAI key's speech and a compatible API's of the OpenAI Speech
//! shape, against a scripted vendor built from OpenAI's Create speech reference
//! (developers.openai.com/api/reference/resources/audio/subresources/speech/methods/create):
//! the body with a built-in voice by name and a custom one as `{ "id" }`, the bytes answered
//! as the file; the built-in voices the catalog names, as OpenAI lists none; a compatible
//! API's `GET …/audio/voices`, and none where the server has no such route. No test calls a
//! vendor.

use std::sync::Arc;

use demi_provider_common::{
    Provider, Secret, VendorPolicy,
    audio::{AudioFormatAsk, AudioMaker, AudioRequest, Voice},
    media::{MediaModelInfo, MediaVendor},
    media_catalog::Catalog,
    testing::{FixedClock, MockResponse, MockVendor},
};
use demi_provider_openai_api::{OpenAiConfig, OpenAiProvider, compatible_audio};
use demi_shared_types::{AudioKind, AudioShape, WireApi};
use serde_json::{Value, json};

use crate::NOW;

fn json_answer(status: u16, body: Value) -> MockResponse {
    MockResponse::status(status).header("content-type", "application/json").chunk(body.to_string())
}

fn speech(model: &str, voice: &str) -> AudioRequest {
    AudioRequest {
        kind: AudioKind::Speech,
        model: model.into(),
        text: "Welcome aboard.".into(),
        voice: Some(voice.into()),
        instructions: Some("Cheerful".into()),
        speed: Some(1.25),
        duration: None,
        instrumental: false,
        seed: None,
        looped: false,
        format: AudioFormatAsk::Wav,
    }
}

const CATALOG: &[u8] = br#"{ "updated": "2026-10-01T00:00:00Z", "models": [
    { "vendor": "openai", "id": "gpt-4o-mini-tts", "kind": "audio", "name": "GPT-4o Mini TTS",
      "audio": { "makes": ["speech"], "voices": ["alloy", "coral", "marin"] } },
    { "vendor": "openai", "id": "gpt-image-2", "kind": "image", "name": "GPT Image 2" }
] }"#;

/// Planted defects this catches: OpenAI's speech taken as having a default
/// voice; a custom voice sent by name, or a built-in
/// one as an object; `response_format` or `instructions` left out; the
/// listing keeping a model the catalog does not know as audio; the voices
/// not read from the catalog or not narrowed by the search.
// One scripted vendor, two requests: milliseconds.
#[tokio::test]
async fn an_openai_key_speaks_with_a_built_in_or_custom_voice_and_lists_the_catalogs_voices() {
    let vendor = MockVendor::start().await;
    vendor.respond_at("/v1/models", json_answer(200, json!({ "data": [
        { "id": "gpt-4o-mini-tts", "created": 1742428800 }, { "id": "gpt-image-2" }, { "id": "gpt-5" }
    ] })));
    vendor.respond_at("/v1/audio/speech", MockResponse::status(200).chunk("RIFF-one"));
    vendor.respond_at("/v1/audio/speech", MockResponse::status(200).chunk("RIFF-two"));
    let config = OpenAiConfig {
        api_key: Some(Secret::try_from("sk-test".to_owned()).unwrap()),
        base_url: Some(vendor.url("/v1").parse().unwrap()),
        wire: WireApi::Responses,
        policy: VendorPolicy::default(),
        vendor_id: Some("openai".into()),
        makes_images: false,
        makes_videos: false,
        makes_audio: true,
    };
    let provider = OpenAiProvider::new(config, Arc::new(FixedClock(NOW.parse().unwrap())));
    let maker = provider.audio().expect("an OpenAI key makes speech");
    // OpenAI requires `voice` and has no default: a call without one fails
    // before it is sent.
    assert_eq!((maker.vendor(), maker.shape(), maker.needs_voice()), (Some(MediaVendor::OpenAi), AudioShape::OpenaiSpeech, true));
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());
    let catalog = Catalog::decode(CATALOG).unwrap();

    let models = maker.models(&http, &catalog).await.unwrap();
    assert_eq!(models.iter().map(|model| model.id.as_str()).collect::<Vec<_>>(), ["gpt-4o-mini-tts"]);
    let made = maker.make(&http, &catalog, speech("gpt-4o-mini-tts", "coral")).await.result.unwrap();
    assert_eq!((made.bytes.as_ref(), made.pcm), (b"RIFF-one".as_slice(), None));
    maker.make(&http, &catalog, speech("gpt-4o-mini-tts", "voice_1234")).await.result.unwrap();
    let sent = vendor.requests();
    assert_eq!(sent[1].header("authorization"), Some("Bearer sk-test"));
    assert_eq!(
        sent[1].json(),
        json!({ "model": "gpt-4o-mini-tts", "input": "Welcome aboard.", "voice": "coral", "instructions": "Cheerful",
                "speed": 1.25, "response_format": "wav" })
    );
    assert_eq!(sent[2].json()["voice"], json!({ "id": "voice_1234" }));

    let voices = maker.voices(&http, &catalog, "gpt-4o-mini-tts", None).await.unwrap().unwrap();
    assert_eq!(voices, ["alloy", "coral", "marin"].map(Voice::named));
    let found = maker.voices(&http, &catalog, "gpt-4o-mini-tts", Some("MAR")).await.unwrap().unwrap();
    assert_eq!(found, [Voice::named("marin")]);
    assert_eq!(vendor.requests().len(), 3, "OpenAI's voices are the catalog's, read with no request");
}

/// Planted defects this catches: a compatible API's voices taken from the
/// catalog; vLLM-Omni's and SGLang-Omni's `uploaded_voices` left out or
/// listed twice; a listed object with both `id` and `name`, or with
/// `display_name` and `language`, failing the listing; a server without
/// `/audio/voices` read as a failure rather than as listing none; a
/// compatible server taken to need a voice, which it does not; its models
/// not marked as speech.
// One scripted vendor, four requests: milliseconds.
#[tokio::test]
async fn a_compatible_api_lists_the_voices_it_serves_in_each_servers_shape_and_none_without_the_route() {
    let vendor = MockVendor::start().await;
    // vLLM-Omni's documented answer: its presets with the uploaded one among
    // them, then the uploaded voices with what they were uploaded with.
    vendor.respond_at("/v1/audio/voices", json_answer(200, json!({
        "voices": ["aiden", "custom_voice_1"],
        "uploaded_voices": [{ "name": "custom_voice_1", "consent": "user_consent_id", "created_at": 1738660000,
            "file_size": 1024000, "mime_type": "audio/wav", "speaker_description": "warm narrator" }]
    })));
    // SGLang-Omni's: its presets, and an uploaded voice with no description.
    vendor.respond_at("/v1/audio/voices", json_answer(200, json!({
        "voices": ["default", "mine"], "uploaded_voices": [{ "name": "mine", "consent": "c", "created_at": 1,
            "file_size": 2, "mime_type": "audio/wav" }], "cache_stats": { "hits": 0 }
    })));
    // Other servers' objects.
    vendor.respond_at("/v1/audio/voices", json_answer(200, json!({ "voices": [
        { "id": "v1", "name": "Vivian" },
        { "id": "ryan", "display_name": "Ryan", "language": "English", "description": "Warm" }
    ] })));
    vendor.respond_at("/v1/audio/voices", MockResponse::status(404).chunk("Not Found"));
    let maker: Arc<dyn AudioMaker> = compatible_audio(
        "Local".into(),
        None,
        &vendor.url("/v1").parse().unwrap(),
        vec![MediaModelInfo::id("qwen3-tts")],
        Arc::new(FixedClock(NOW.parse().unwrap())),
    );
    let http = demi_provider_common::ProviderHttp::new(reqwest::Client::new());
    let catalog = Catalog::decode(CATALOG).unwrap();

    assert!(!maker.needs_voice());
    let models = maker.models(&http, &catalog).await.unwrap();
    assert_eq!(models[0].kinds, [AudioKind::Speech]);
    let voices = maker.voices(&http, &catalog, "qwen3-tts", None).await.unwrap().unwrap();
    assert_eq!(
        voices,
        [
            Voice { own: None, ..Voice::named("aiden") },
            Voice { own: Some(true), description: Some("warm narrator".into()), ..Voice::named("custom_voice_1") },
        ]
    );
    let voices = maker.voices(&http, &catalog, "qwen3-tts", None).await.unwrap().unwrap();
    assert_eq!(voices, [Voice { own: None, ..Voice::named("default") }, Voice { own: Some(true), ..Voice::named("mine") }]);
    let voices = maker.voices(&http, &catalog, "qwen3-tts", None).await.unwrap().unwrap();
    assert_eq!(
        voices,
        [
            Voice { id: "v1".into(), name: Some("Vivian".into()), languages: Vec::new(), description: None, own: None },
            Voice { id: "ryan".into(), name: Some("Ryan".into()), languages: vec!["English".into()],
                    description: Some("Warm".into()), own: None },
        ]
    );
    assert_eq!(maker.voices(&http, &catalog, "qwen3-tts", None).await.unwrap(), None);
    assert_eq!(vendor.requests()[0].header("authorization"), None, "a compatible API without a key sends none");
}
