//! A media call's answer and its redirects (`media.md`, `audio.md`
//! § Vendors), against scripted servers: a file read within 256 MiB as it
//! arrives, and a vendor's own key header sent only to the origin the call
//! started at, whichever way a redirect leads.

use bytes::Bytes;
use demi_provider_common::media::{ANSWER_LIMIT, MediaCall, MediaCredential, MediaFailure};
use demi_provider_common::testing::{FixedClock, MockResponse, MockVendor};
use demi_provider_common::{ProviderHttp, read_http_failure};
use http::{HeaderMap, HeaderName, HeaderValue};

fn call(clock: &FixedClock) -> MediaCall<'_> {
    MediaCall {
        label: "ElevenLabs",
        credential: MediaCredential::Key { refused: |status, _| status == http::StatusCode::UNAUTHORIZED },
        reader: read_http_failure,
        used_up: |failure| failure,
        clock,
        quota: None,
    }
}

fn key() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(HeaderName::from_static("xi-api-key"), HeaderValue::from_static("xi-key"));
    headers
}

fn clock() -> FixedClock {
    FixedClock("2026-10-18T12:00:00Z".parse().unwrap())
}

/// A speech answered with more than 256 MiB fails as it arrives, without a
/// length announced first, rather than being kept whole.
/// Planted defect this catches: the answer read whole, as
/// `response.bytes()` reads it.
// One scripted server streaming 257 MiB of one shared chunk: about a
// second, and 256 MiB of memory at most.
#[tokio::test]
async fn a_file_answer_past_the_limit_fails_as_it_arrives() {
    let vendor = MockVendor::start().await;
    let mebibyte = Bytes::from(vec![0_u8; 1024 * 1024]);
    let mut response = MockResponse::status(200);
    for _ in 0..=ANSWER_LIMIT / mebibyte.len() {
        response = response.chunk(mebibyte.clone());
    }
    vendor.respond_at("/v1/text-to-speech/george", response);
    let url = vendor.url("/v1/text-to-speech/george").parse().unwrap();
    let clock = clock();

    let answer = call(&clock)
        .post_json_for_file(&ProviderHttp::new(reqwest::Client::new()), &url, Bytes::from_static(b"{}"), |_| {
            std::future::ready(Ok((key(), None)))
        })
        .await;

    match answer.result {
        Err(MediaFailure::Unanswered { cause, code: None }) => assert_eq!(cause, "its answer is over 256 MiB"),
        other => panic!("not refused for its size: {other:?}"),
    }
}

/// A call's key header goes to its own origin, and never to another a
/// redirect leads to: a 307 keeps the `POST` and its body without the key,
/// a 303 turns into a `GET`, and a redirect within the origin keeps it.
/// Planted defect this catches: redirects followed by the client, which
/// drops only `Authorization` across hosts and so sends `xi-api-key` or
/// `x-goog-api-key` on to whoever the vendor's answer names.
// Two scripted servers, five requests: milliseconds.
#[tokio::test]
async fn a_key_header_goes_only_to_the_origin_the_call_started_at() {
    let vendor = MockVendor::start().await;
    let storage = MockVendor::start().await;
    vendor.respond_at("/v1/music", MockResponse::status(307).header("location", &storage.url("/stored/a")));
    storage.respond_at("/stored/a", MockResponse::status(200).chunk("ID3"));
    vendor.respond_at("/v1/models", MockResponse::status(302).header("location", "/v1/models/all"));
    vendor.respond_at("/v1/models/all", MockResponse::status(303).header("location", &storage.url("/listing")));
    storage.respond_at("/listing", MockResponse::status(200).chunk("[]"));
    let http = ProviderHttp::new(reqwest::Client::new());
    let clock = clock();

    let music = call(&clock)
        .post_json_for_file(&http, &vendor.url("/v1/music").parse().unwrap(), Bytes::from_static(b"{\"prompt\":\"a\"}"), |_| {
            std::future::ready(Ok((key(), None)))
        })
        .await;
    assert_eq!(music.result.unwrap(), Bytes::from_static(b"ID3"));
    let listing = call(&clock).get(&http, &vendor.url("/v1/models").parse().unwrap(), key()).await.unwrap();
    assert_eq!(listing, Bytes::from_static(b"[]"));

    let at_vendor = vendor.requests();
    let at_storage = storage.requests();
    assert_eq!(at_vendor.iter().map(|request| request.header("xi-api-key")).collect::<Vec<_>>(), [Some("xi-key"); 3]);
    assert_eq!(at_vendor[2].uri.path(), "/v1/models/all");
    let stored = &at_storage[0];
    assert_eq!((stored.method.as_str(), stored.header("xi-api-key"), stored.body.as_ref()), ("POST", None, b"{\"prompt\":\"a\"}".as_slice()));
    assert_eq!((at_storage[1].method.as_str(), at_storage[1].header("xi-api-key")), ("GET", None));
}
