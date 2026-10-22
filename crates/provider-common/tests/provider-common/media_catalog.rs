//! The catalog of image and video models
//! (`crates/provider-common/src/media_catalog.rs`), against a scripted server.
//!
//! The tests cover the file's decoding, the repository's own file, the download
//! of the build's branch with its newer-of rule, and the image models a
//! subscription and a key's listing read from it. No test reaches GitHub.

use std::sync::Arc;
use std::time::Duration;

use demi_provider_common::media_catalog::{
    Catalog, MediaCatalog,
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

/// `future`, with the runtime kept busy while it waits, so that a paused clock
/// stands still until the test moves it.
///
/// A download under way waits on the network, which an idle runtime's paused
/// clock would skip past to the next timer, the download's deadline among them.
/// Each `yield_now` waits for the runtime's next poll of the network. What has
/// not happened after ten thousand of them will not happen while the clock
/// stands, and fails the test rather than hang it.
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
