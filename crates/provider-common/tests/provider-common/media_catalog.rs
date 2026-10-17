//! The catalog of image and video models (`media-models.md`): the file's
//! decoding, the repository's own file, the download of the build's
//! branch with its newer-of rule, and the image models a subscription and
//! a key's listing read from it (`images.md` § Vendors), against a
//! scripted server. No test reaches GitHub.

use std::sync::Arc;
use std::time::Duration;

use demi_provider_common::media::{MediaModelInfo, subscription_models};
use demi_provider_common::media_catalog::{
    AudioOutput, Catalog, Currency, DurationRange, ListedVoices, MediaCatalog, MediaKind, Modality, PriceUnit, Subscription, Voices, branch_url,
};
use demi_provider_common::testing::{MockResponse, MockVendor};
use serde_json::{Value, json};
use tokio::runtime::Handle;
use tokio::time::Instant;

/// A catalog updated at `updated` whose one model is `id`.
fn catalog(updated: &str, id: &str) -> Value {
    json!({ "updated": updated, "models": [
        { "vendor": "openai", "id": id, "kind": "image", "name": "Picture" }
    ] })
}

fn decoded(document: &Value) -> Arc<Catalog> {
    Arc::new(Catalog::decode(document.to_string().as_bytes()).unwrap())
}

fn served(document: &Value) -> MockResponse {
    MockResponse::status(200)
        .header("content-type", "application/json")
        .header("etag", "\"v1\"")
        .chunk(document.to_string())
}

fn ids(catalog: &Catalog) -> Vec<&str> {
    catalog.models.iter().map(|model| model.id.as_str()).collect()
}

// Decoding in memory: microseconds.
//
// Planted defects: an unknown field or kind that refuses the file; a
// missing `name` accepted; a size that is not WxH, or has a side of 0,
// accepted; content after the document accepted; an alias of an alias, or
// of a model the file lacks, accepted.
#[test]
fn a_catalog_decodes_ignoring_what_it_does_not_know_and_refuses_a_missing_required_field() {
    let read = Catalog::decode(
        json!({ "updated": "2026-10-01T00:00:00Z", "publisher": "demi", "models": [
            { "vendor": "bytedance", "id": "seedream-5-1", "kind": "image", "name": "Seedream 5.1",
              "image": { "sizes": ["2048x2048"], "styles": ["anime"] }, "license": "commercial" },
            { "vendor": "acme", "id": "chair-1", "kind": "3d", "name": "Chair 1" }
        ] })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    assert_eq!(ids(&read), ["seedream-5-1", "chair-1"]);
    assert_eq!(read.models[1].kind, MediaKind::Other);
    let size = read.models[0].image.as_ref().unwrap().sizes.as_ref().unwrap()[0];
    assert_eq!((size.width, size.height), (2048, 2048));

    let nameless = json!({ "updated": "2026-10-01T00:00:00Z", "models": [
        { "vendor": "openai", "id": "x", "kind": "image" }
    ] });
    let refused = Catalog::decode(nameless.to_string().as_bytes()).unwrap_err();
    assert!(refused.contains("models[0]") && refused.contains("name"), "{refused}");
    let sizeless = json!({ "updated": "2026-10-01T00:00:00Z", "models": [
        { "vendor": "openai", "id": "x", "kind": "image", "name": "X", "image": { "sizes": ["large"] } }
    ] });
    let refused = Catalog::decode(sizeless.to_string().as_bytes()).unwrap_err();
    assert!(refused.contains("large"), "{refused}");
    let flat = json!({ "updated": "2026-10-01T00:00:00Z", "models": [
        { "vendor": "openai", "id": "x", "kind": "image", "name": "X", "image": { "sizes": ["0x1024"] } }
    ] });
    let refused = Catalog::decode(flat.to_string().as_bytes()).unwrap_err();
    assert!(refused.contains("0x1024"), "{refused}");

    let mut trailed = catalog("2026-10-01T00:00:00Z", "x").to_string();
    trailed.push_str(" {}");
    assert!(Catalog::decode(trailed.as_bytes()).is_err());

    let aliases = |named: &str| {
        json!({ "updated": "2026-10-01T00:00:00Z", "models": [
            { "vendor": "openai", "id": "pic-2026", "kind": "image", "name": "Pic" },
            { "vendor": "openai", "id": "pic", "kind": "image", "name": "Pic", "aliasOf": "pic-2026" },
            { "vendor": "openai", "id": "pic-latest", "kind": "image", "name": "Pic", "aliasOf": named }
        ] })
        .to_string()
    };
    assert!(Catalog::decode(aliases("pic-2026").as_bytes()).is_ok());
    let refused = Catalog::decode(aliases("pic").as_bytes()).unwrap_err();
    assert!(refused.contains("models[2].aliasOf") && refused.contains("pic is an alias"), "{refused}");
    let refused = Catalog::decode(aliases("pic-2025").as_bytes()).unwrap_err();
    assert!(refused.contains("models[2].aliasOf") && refused.contains("no model pic-2025"), "{refused}");
}

// Decoding in memory: microseconds.
//
// Planted defects: an audio model read as an unknown kind; voices that the
// vendor lists refused, or named voices read as listed; a unit, a currency
// or a duration of a rate misread; an unknown unit, currency or output
// refusing the file.
#[test]
fn an_audio_model_decodes_with_its_voices_durations_and_rates() {
    let read = Catalog::decode(
        json!({ "updated": "2026-10-01T00:00:00Z", "models": [
            { "vendor": "elevenlabs", "id": "voice-1", "kind": "audio", "name": "Voice 1",
              "audio": { "makes": ["speech"], "voices": "listed", "languages": ["en", "de"], "instructions": true,
                         "formats": ["mp3", "wav"] },
              "price": { "rates": [{ "unit": "millionCharacters", "amount": 80.0, "currency": "usd" }] } },
            { "vendor": "openai", "id": "voice-2", "kind": "audio", "name": "Voice 2",
              "audio": { "makes": ["speech"], "voices": ["alloy", "marin"] } },
            { "vendor": "elevenlabs", "id": "sound-1", "kind": "audio", "name": "Sound 1",
              "audio": { "makes": ["effect", "ringtone"], "durationRange": { "min": 0.5, "max": 30 }, "loops": true },
              "price": { "rates": [{ "unit": "minute", "amount": 0.12, "currency": "usd" }] } },
            { "vendor": "google", "id": "song-1", "kind": "audio", "name": "Song 1",
              "audio": { "makes": ["music"], "durations": [30], "durationRange": { "max": 120 }, "lyrics": true, "instrumental": true },
              "price": { "rates": [
                  { "unit": "millionInputTokens", "amount": 1.5, "currency": "usd" },
                  { "unit": "millionCachedInputTokens", "modality": "image", "amount": 2, "currency": "usd" },
                  { "unit": "millionOutputTokens", "amount": 3, "currency": "cny", "duration": 30 },
                  { "unit": "video", "amount": 0.28, "currency": "usd", "resolution": "768P", "duration": 6 }
              ] } },
            { "vendor": "acme", "id": "song-2", "kind": "audio", "name": "Song 2",
              "price": { "rates": [{ "unit": "bar", "amount": 1, "currency": "eur" }] } }
        ] })
        .to_string()
        .as_bytes(),
    )
    .unwrap();
    assert!(read.models.iter().all(|model| model.kind == MediaKind::Audio));
    let audio = |index: usize| read.models[index].audio.as_ref().unwrap();
    assert_eq!(audio(0).voices, Some(Voices::Listed(ListedVoices::Listed)));
    assert_eq!(audio(1).voices, Some(Voices::Named(vec!["alloy".into(), "marin".into()])));
    assert_eq!(audio(2).makes.as_deref(), Some(&[AudioOutput::Effect, AudioOutput::Other][..]));
    assert_eq!(audio(2).duration_range, Some(DurationRange { min: Some(0.5), max: Some(30.0) }));
    assert_eq!(audio(3).durations.as_deref(), Some(&[30][..]));
    assert_eq!(audio(3).duration_range, Some(DurationRange { min: None, max: Some(120.0) }));
    let rates: Vec<(PriceUnit, Option<Modality>, Currency, Option<f64>)> = read
        .models
        .iter()
        .filter_map(|model| model.price.as_ref())
        .flat_map(|price| &price.rates)
        .map(|rate| (rate.unit, rate.modality, rate.currency, rate.duration))
        .collect();
    assert_eq!(
        rates,
        [
            (PriceUnit::MillionCharacters, None, Currency::Usd, None),
            (PriceUnit::Minute, None, Currency::Usd, None),
            (PriceUnit::MillionInputTokens, None, Currency::Usd, None),
            (PriceUnit::MillionCachedInputTokens, Some(Modality::Image), Currency::Usd, None),
            (PriceUnit::MillionOutputTokens, None, Currency::Cny, Some(30.0)),
            (PriceUnit::Video, None, Currency::Usd, Some(6.0)),
            (PriceUnit::Other, None, Currency::Other, None),
        ]
    );
}

// Reads one file of the repository: milliseconds.
//
// Planted defect: an edit of `catalog/media/v1/models.json` the backend
// would refuse.
#[test]
fn the_repositorys_catalog_decodes() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../catalog/media/v1/models.json");
    let catalog = Catalog::decode(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(&catalog, &*Catalog::carried());
}

// A scripted server and a few downloads: milliseconds.
//
// Planted defects: the older of two files used; a download that fails or
// does not decode replacing the catalog; a second download sent without
// the first one's ETag.
#[tokio::test]
async fn a_download_is_used_only_when_newer_and_a_failed_or_older_one_keeps_the_catalog() {
    let vendor = MockVendor::start().await;
    let url = vendor.url("/wspl/demi/main/catalog/media/v1/models.json").parse().unwrap();
    let carried = decoded(&catalog("2026-10-01T00:00:00Z", "carried"));
    let media = MediaCatalog::new(reqwest::Client::new(), Some(url), carried);
    assert_eq!(ids(&media.current()), ["carried"]);

    // A newer file replaces the release's copy.
    vendor.respond(served(&catalog("2026-10-05T00:00:00Z", "newer")));
    media.refresh().await;
    assert_eq!(ids(&media.current()), ["newer"]);

    // An unchanged file is confirmed with its ETag and changes nothing.
    vendor.respond(MockResponse::status(304));
    media.refresh().await;
    assert_eq!(vendor.requests()[1].header("if-none-match"), Some("\"v1\""));
    assert_eq!(ids(&media.current()), ["newer"]);

    // A failure, a file that does not decode and an older file each keep
    // the catalog in use.
    vendor.respond(MockResponse::status(500));
    vendor.respond(served(&json!({ "updated": "2026-10-09T00:00:00Z", "models": [{ "id": "x" }] })));
    vendor.respond(served(&catalog("2026-10-03T00:00:00Z", "older")));
    for _ in 0..3 {
        media.refresh().await;
        assert_eq!(ids(&media.current()), ["newer"]);
    }
    assert_eq!(vendor.requests().len(), 5);
}

/// `future`, with the runtime kept busy while it waits, so that a paused
/// clock stands still until the test moves it: a download under way waits
/// on the network, which an idle runtime's paused clock would skip past to
/// the next timer, the download's deadline among them. Each `yield_now`
/// waits for the runtime's next poll of the network; what has not happened
/// after ten thousand of them will not happen while the clock stands, and
/// fails the test rather than hang it.
async fn busy<F: std::future::Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    for _ in 0..10_000 {
        tokio::select! {
            biased;
            output = &mut future => return output,
            () = tokio::task::yield_now() => {}
        }
    }
    panic!("what the test waits for does not happen while the clock stands");
}

/// Lets the runtime finish the network work under way while the clock
/// stands: a request a task sent before reaches the vendor within a few of
/// these hundred polls of the network.
async fn settle() {
    for _ in 0..100 {
        tokio::task::yield_now().await;
    }
}

// A scripted server on a paused clock, which the test moves a day at a
// time: milliseconds.
//
// Planted defects: no download at the start; a download more or less often
// than daily; a failed download retried before the next day; a download
// under way that outlives `close`.
#[tokio::test(start_paused = true)]
async fn a_started_catalog_downloads_at_once_then_daily_and_closing_ends_a_download_under_way() {
    const DAY: Duration = Duration::from_secs(24 * 60 * 60);
    let vendor = MockVendor::start().await;
    let url = vendor.url("/wspl/demi/main/catalog/media/v1/models.json").parse().unwrap();
    let media = MediaCatalog::new(reqwest::Client::new(), Some(url), decoded(&catalog("2026-10-01T00:00:00Z", "carried")));
    let mut changes = media.subscribe();
    let started = Instant::now();
    vendor.respond(served(&catalog("2026-10-02T00:00:00Z", "first")));
    // A refusal whose body the catalog leaves unread, so the vendor sees
    // when the catalog is done with it.
    vendor.respond(MockResponse::status(503).chunk("unavailable").stay_open());
    vendor.respond(served(&catalog("2026-10-03T00:00:00Z", "third")));
    media.start(&Handle::current());
    busy(changes.changed()).await.unwrap();
    assert_eq!(ids(&changes.borrow_and_update()), ["first"]);

    // The next day's download fails, and the one after it waits for the
    // day after.
    tokio::time::advance(DAY).await;
    busy(vendor.received(2)).await;
    busy(vendor.disconnected()).await;
    settle().await;
    assert_eq!(vendor.requests().len(), 2, "a failed download is not retried before the next day");
    tokio::time::advance(DAY).await;
    busy(changes.changed()).await.unwrap();
    assert_eq!(ids(&changes.borrow_and_update()), ["third"]);
    let days: Vec<Duration> = vendor.requests().iter().map(|request| request.at - started).collect();
    assert_eq!(days, [Duration::ZERO, DAY, 2 * DAY]);

    // A download under way when the catalog closes is dropped.
    vendor.respond(MockResponse::status(200).header("content-type", "application/json").chunk("{").stay_open());
    tokio::time::advance(DAY).await;
    busy(vendor.received(4)).await;
    media.close();
    busy(vendor.disconnected()).await;
    assert_eq!(ids(&media.current()), ["third"]);
}

// No server at all: microseconds.
//
// Planted defect: a build from no branch that downloads, or reads `main`.
#[tokio::test]
async fn a_build_from_no_branch_downloads_nothing_and_keeps_its_copy() {
    assert_eq!(branch_url(None), None);
    assert_eq!(
        branch_url(Some("feat/demi-next")).unwrap().as_str(),
        "https://raw.githubusercontent.com/wspl/demi/feat/demi-next/catalog/media/v1/models.json"
    );
    let carried = decoded(&catalog("2026-10-01T00:00:00Z", "carried"));
    let media = MediaCatalog::new(reqwest::Client::new(), None, carried.clone());
    media.start(&Handle::current());
    media.refresh().await;
    assert!(Arc::ptr_eq(&media.current(), &carried));
    media.close();
}

fn listed_ids(models: &[MediaModelInfo]) -> Vec<&str> {
    models.iter().map(|model| model.id.as_str()).collect()
}

// Reads a catalog in memory: microseconds. What `demi image models` says
// of each model, an alias's facts included, is the backend's scenario's.
//
// Planted defect: a video model, or another subscription's, listed as an
// image model.
#[test]
fn a_subscription_lists_the_catalogs_image_models_for_it() {
    let catalog = decoded(&json!({ "updated": "2026-10-01T00:00:00Z", "models": [
        { "vendor": "openai", "id": "pic", "kind": "image", "name": "Pic", "description": "Best pictures",
          "aliasOf": "pic-2026-09-08", "subscriptions": ["chatgpt"] },
        { "vendor": "openai", "id": "pic-2026-09-08", "kind": "image", "name": "Pic", "released": "2026-09-08",
          "description": "Dated snapshot of pic", "subscriptions": ["chatgpt"],
          "image": { "customSize": { "multipleOf": 16, "maxEdge": 3840, "maxRatio": 3, "minPixels": 655360, "maxPixels": 8294400 },
                     "transparent": true, "edits": true } },
        { "vendor": "openai", "id": "pic-1", "kind": "image", "name": "Pic 1", "released": "2025-04-23",
          "subscriptions": ["chatgpt"], "image": { "sizes": ["1024x1024", "1536x1024", "1024x1536"] } },
        { "vendor": "openai", "id": "pic-latest", "kind": "image", "name": "Pic", "aliasOf": "pic-1",
          "subscriptions": ["chatgpt"] },
        { "vendor": "openai", "id": "clip", "kind": "video", "name": "Clip", "subscriptions": ["chatgpt"] },
        { "vendor": "xai", "id": "imagine", "kind": "image", "name": "Imagine", "subscriptions": ["grok"],
          "image": { "aspectRatios": ["1:1", "16:9", "9:19.5"], "resolutions": ["1k", "2k"], "edits": true, "maxInputImages": 5 } }
    ] }));
    assert_eq!(listed_ids(&subscription_models(&catalog, Subscription::Chatgpt, MediaKind::Image)), ["pic", "pic-2026-09-08", "pic-1", "pic-latest"]);
    assert_eq!(listed_ids(&subscription_models(&catalog, Subscription::Grok, MediaKind::Image)), ["imagine"]);
}
