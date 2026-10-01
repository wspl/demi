//! What the provider says about itself: its catalog.

use demi_provider_common::testing::{MockVendor, assert_built_in_catalog};

use crate::provider_at;

#[tokio::test]
async fn the_catalog_is_the_built_in_directory_served_without_a_request() {
    let vendor = MockVendor::start().await;
    assert_built_in_catalog(&provider_at(&vendor, "/v1beta"), &vendor).await;
}
