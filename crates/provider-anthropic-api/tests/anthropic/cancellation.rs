//! Cancelling a run: it stops at once, drops its connection and ends without
//! a further event (`providers.md` § A run).

use demi_provider::{
    ProviderEvent,
    testing::{MockResponse, MockVendor, inference_request},
};
use futures_util::StreamExt;

use crate::{run, runtime};

/// A response that streams text and then stays open until the client leaves.
fn open_stream() -> MockResponse {
    MockResponse::event_stream(concat!(
        "data: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":3}}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hel\"}}\n\n",
    ))
    .stay_open()
}

#[tokio::test]
async fn a_cancelled_request_sends_nothing_and_ends_without_an_event() {
    let vendor = MockVendor::start().await;
    let request = inference_request();
    request.cancel.cancel();
    assert!(run(runtime(&vendor).as_mut(), request).await.is_empty());
    assert!(vendor.requests().is_empty());
}

#[tokio::test]
async fn cancelling_mid_stream_ends_the_run_without_an_event_and_drops_the_connection() {
    let vendor = MockVendor::start().await;
    vendor.respond(open_stream());
    let mut runtime = runtime(&vendor);
    let request = inference_request();
    let cancel = request.cancel.clone();
    let mut events = runtime.run(request);
    assert_eq!(events.next().await, Some(ProviderEvent::TextDelta("hel".into())));
    cancel.cancel();
    assert_eq!(events.next().await, None);
    vendor.disconnected().await;
}

#[tokio::test]
async fn dropping_the_run_mid_stream_drops_the_connection() {
    let vendor = MockVendor::start().await;
    vendor.respond(open_stream());
    let mut runtime = runtime(&vendor);
    let mut events = runtime.run(inference_request());
    assert_eq!(events.next().await, Some(ProviderEvent::TextDelta("hel".into())));
    drop(events);
    vendor.disconnected().await;
}
