//! Cancelling a run: it stops at once, drops its connection and ends without
//! a further event (`providers.md` § A run).

use demi_provider::{
    ProviderEvent,
    testing::{MockResponse, MockVendor, inference_request},
};
use demi_provider_openai_api::{VendorPolicy, WireApi};
use futures_util::StreamExt;

use crate::{run, runtime};

#[tokio::test]
async fn a_cancelled_request_sends_nothing_and_ends_without_an_event() {
    let vendor = MockVendor::start().await;
    let request = inference_request();
    request.cancel.cancel();
    assert!(run(runtime(&vendor, WireApi::Responses, VendorPolicy::default()).as_mut(), request).await.is_empty());
    assert!(vendor.requests().is_empty());
}

#[tokio::test]
async fn cancelling_mid_stream_ends_the_run_without_an_event_and_drops_the_connection() {
    let open = [
        (WireApi::Responses, "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hel\"}\n\n"),
        (WireApi::ChatCompletions, "data: {\"choices\":[{\"delta\":{\"content\":\"hel\"}}]}\n\n"),
    ];
    for (wire, first) in open {
        let vendor = MockVendor::start().await;
        vendor.respond(MockResponse::event_stream(first).stay_open());
        let mut runtime = runtime(&vendor, wire, VendorPolicy::default());
        let request = inference_request();
        let cancel = request.cancel.clone();
        let mut events = runtime.run(request);
        assert_eq!(events.next().await, Some(ProviderEvent::TextDelta("hel".into())));
        cancel.cancel();
        assert_eq!(events.next().await, None);
        vendor.disconnected().await;
    }
}
