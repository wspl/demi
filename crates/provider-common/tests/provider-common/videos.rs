//! A finished video's download (`videos.md` § The job), against a scripted
//! server.

use bytes::Bytes;
use demi_provider_common::ProviderHttp;
use demi_provider_common::media::MediaFailure;
use demi_provider_common::testing::{FixedClock, MockResponse, MockVendor};
use demi_provider_common::videos::{DOWNLOAD_LIMIT, download};
use http::HeaderMap;

/// A video past 256 MiB fails as it arrives, without a length announced
/// first, rather than being kept whole.
/// Planted defect this catches: the size read only from a
/// `Content-Length`, which this answer does not send.
// One scripted server streaming 257 MiB of one shared chunk: under a
// second, and 256 MiB of memory at most.
#[tokio::test]
async fn a_video_past_the_limit_fails_as_it_arrives() {
    let vendor = MockVendor::start().await;
    let mebibyte = Bytes::from(vec![0_u8; 1024 * 1024]);
    let mut response = MockResponse::status(200);
    for _ in 0..=DOWNLOAD_LIMIT / mebibyte.len() {
        response = response.chunk(mebibyte.clone());
    }
    vendor.respond_at("/video.mp4", response);
    let url = vendor.url("/video.mp4").parse().unwrap();
    let clock = FixedClock("2026-10-17T12:00:00Z".parse().unwrap());

    let answer = download("Grok", &clock, &ProviderHttp::new(reqwest::Client::new()), &url, &url, HeaderMap::new()).await;

    match answer.result {
        Err(MediaFailure::Unanswered { cause, code: None }) => assert_eq!(cause, "its video is over 256 MiB"),
        other => panic!("not refused for its size: {other:?}"),
    }
}
