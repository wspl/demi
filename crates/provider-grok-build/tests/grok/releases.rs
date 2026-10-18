//! The Grok CLI version every request names (`providers.md` § Endpoints):
//! the newest stable release the pointer names, kept as Codex's release is.

use std::sync::Arc;

use demi_provider_common::{
    Provider,
    testing::{FixedClock, ManualClock, MockResponse, MockVendor, guarded},
};
use demi_shared_types::Clock;
use serde_json::json;
use tokio::sync::watch;

use crate::{ACCOUNT, NOW, STABLE, json_answer, pool_with, provider_at, releases_at, secret};

const MODELS: &str = "/v1/models";

/// What a request that must not wait for the pointer is guarded as.
const UNWAITED: &str = "a request that does not wait for the pointer";

/// The version the provider sends before it has read the pointer.
const BUILT_IN: &str = "1.0.62";

/// The pointer's answer: `text`, as plain text.
fn pointer(text: &str) -> MockResponse {
    MockResponse::status(200)
        .header("content-type", "text/plain")
        .chunk(text.to_owned())
}

/// The `x-grok-client-version` of each catalog request so far.
fn sent_versions(vendor: &MockVendor) -> Vec<String> {
    vendor
        .requests()
        .iter()
        .filter(|request| request.uri.path() == MODELS)
        .map(|request| request.header("x-grok-client-version").unwrap().to_owned())
        .collect()
}

/// How many times the pointer was read.
fn pointer_reads(vendor: &MockVendor) -> usize {
    let requests = vendor.requests();
    requests.iter().filter(|request| request.uri.path() == STABLE).count()
}

/// Reads the catalog until a request names `version`, which a read of the
/// pointer in the background brings.
async fn catalog_at(provider: &impl Provider, vendor: &MockVendor, version: &str) {
    let polled = async {
        loop {
            vendor.respond_at(MODELS, json_answer(200, json!([{ "id": "grok-test" }])));
            provider.list_models().await.unwrap();
            if sent_versions(vendor).last().map(String::as_str) == Some(version) {
                return;
            }
        }
    };
    guarded(&format!("a request naming version {version}"), polled).await;
}

#[tokio::test]
async fn requests_name_the_newest_stable_grok_cli_release_the_pointer_names() {
    let vendor = MockVendor::start().await;
    // The pointer first names a prerelease, which is no answer, then a
    // release on its first line. Its first answer waits until the test lets
    // it go.
    let (open, gate) = watch::channel(false);
    vendor.respond_at(STABLE, pointer("1.0.70-beta.1\n").held(gate));
    vendor.respond_at(STABLE, pointer("1.0.70\nsha256 0123\n"));
    let pool = pool_with(secret(&vendor, json!({}))).await;
    let clock: Arc<dyn Clock> = Arc::new(FixedClock(NOW.parse().unwrap()));
    let releases = releases_at(&vendor, clock.clone());
    let provider = provider_at(&vendor, &pool, Some(ACCOUNT), clock, releases);

    // Before the pointer answers, a request names the built-in version and
    // does not wait for it.
    vendor.respond_at(MODELS, json_answer(200, json!([{ "id": "grok-test" }])));
    guarded(UNWAITED, provider.list_models()).await.unwrap();
    assert_eq!(sent_versions(&vendor), [BUILT_IN]);
    open.send(true).unwrap();

    catalog_at(&provider, &vendor, "1.0.70").await;
    // No request named the prerelease: its read failed, and the pointer was
    // read again.
    let versions = sent_versions(&vendor);
    assert!(
        versions.iter().all(|version| [BUILT_IN, "1.0.70"].contains(&version.as_str())),
        "{versions:?}"
    );
    assert_eq!(pointer_reads(&vendor), 2);

    // The version read is believed: the next request names it and reads the
    // pointer no more.
    vendor.respond_at(MODELS, json_answer(200, json!([{ "id": "grok-test" }])));
    provider.list_models().await.unwrap();
    assert_eq!(sent_versions(&vendor).last().unwrap(), "1.0.70");
    assert_eq!(pointer_reads(&vendor), 2);
}

#[tokio::test]
async fn a_pointer_that_fails_leaves_the_version_last_read_and_is_read_again_after_six_hours() {
    let vendor = MockVendor::start().await;
    let clock = Arc::new(ManualClock::new(NOW.parse().unwrap()));
    let pool = pool_with(secret(&vendor, json!({}))).await;
    let releases = releases_at(&vendor, clock.clone());
    let provider = provider_at(&vendor, &pool, Some(ACCOUNT), clock.clone(), releases);

    // The pointer cannot be reached: a request names the built-in version,
    // without waiting for the read.
    let (open, gate) = watch::channel(false);
    vendor.respond_at(STABLE, MockResponse::status(503).held(gate));
    vendor.respond_at(MODELS, json_answer(200, json!([{ "id": "grok-test" }])));
    guarded(UNWAITED, provider.list_models()).await.unwrap();
    assert_eq!(sent_versions(&vendor), [BUILT_IN]);
    open.send(true).unwrap();

    // Once the pointer answers, a later request names what it named.
    vendor.respond_at(STABLE, pointer("1.0.70\n"));
    catalog_at(&provider, &vendor, "1.0.70").await;
    let reads = pointer_reads(&vendor);

    // Six hours later the version is read again in the background. The
    // pointer answers a line that is no bare version, and requests keep
    // naming the last one read while that read waits and after it fails.
    clock.advance(jiff::SignedDuration::from_hours(6));
    let (open, gate) = watch::channel(false);
    vendor.respond_at(STABLE, pointer("v1.0.71\n").held(gate));
    vendor.respond_at(MODELS, json_answer(200, json!([{ "id": "grok-test" }])));
    let before = vendor.requests().len();
    guarded(UNWAITED, provider.list_models()).await.unwrap();
    // The catalog request and the pointer's.
    vendor.received(before + 2).await;
    assert_eq!(pointer_reads(&vendor), reads + 1);
    assert_eq!(sent_versions(&vendor).last().unwrap(), "1.0.70");
    open.send(true).unwrap();
    vendor.respond_at(STABLE, pointer("1.0.71\n"));
    let before = sent_versions(&vendor).len();
    catalog_at(&provider, &vendor, "1.0.71").await;
    let since = &sent_versions(&vendor)[before..];
    assert!(
        since.iter().all(|version| ["1.0.70", "1.0.71"].contains(&version.as_str())),
        "{since:?}"
    );
}
