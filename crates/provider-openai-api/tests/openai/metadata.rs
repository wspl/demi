//! What the provider says about itself: its catalog, on either wire.

use demi_shared_types::WireApi;
use demi_provider_common::testing::{MockVendor, assert_built_in_catalog};
use demi_provider_openai_api::VendorPolicy;

use crate::provider_at;

#[tokio::test]
async fn the_catalog_is_the_built_in_directory_served_without_a_request() {
    let vendor = MockVendor::start().await;
    for wire in [WireApi::Responses, WireApi::ChatCompletions] {
        let provider = provider_at(&vendor, "/v1", wire, VendorPolicy::default());
        assert_built_in_catalog(&provider, &vendor).await;
    }
}
