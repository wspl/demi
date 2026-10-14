//! What the provider says about itself: how it reads its failure records.

use demi_provider_common::{HttpFailureRecord, Provider, testing::MockVendor};
use demi_shared_types::{FailureSource, ProviderErrorDiagnostics};

use crate::provider_at;

#[tokio::test]
async fn a_failure_record_is_read_by_the_standard_reading() {
    let vendor = MockVendor::start().await;
    let record = HttpFailureRecord {
        status: 429,
        headers: vec![("retry-after".into(), "90".into())],
        body: String::new(),
    };
    let diagnostics = ProviderErrorDiagnostics {
        source: FailureSource::Http,
        client_request_id: None,
        provider_request_id: None,
        provider_response_id: None,
        provider_code: None,
        http_status: Some(429),
        upstream: Some(record.to_json()),
    };
    let facts = provider_at(&vendor, "/v1")
        .read_failure(&diagnostics, "2026-09-18T14:00:00.000Z".parse().unwrap());
    assert_eq!(
        facts.retry_at,
        Some("2026-09-18T14:01:30.000Z".parse().unwrap())
    );
}
