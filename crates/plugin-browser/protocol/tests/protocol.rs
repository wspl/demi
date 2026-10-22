//! The `demi.browser` package's query trees and release records.

use demi_command_package_browser_protocol::{
    browser::{BrowserQuery, BrowserTarget},
    release::BrowserRelease,
};
use serde_json::json;

#[test]
fn a_query_tree_has_one_base_in_every_branch() {
    let query = BrowserQuery::parse(
        r#"{"and": [{"match": {"role": "row"}}, {"match": {"text-match": "Order A"}}],
            "has": {"match": {"role": "button", "name": "Delete"}}, "nth": 0}"#,
    )
    .unwrap();
    let branch = &query.and.as_ref().unwrap()[1];
    let target = BrowserTarget::from(branch.r#match.clone().unwrap());
    assert_eq!(target.text_match.as_deref(), Some("Order A"));
    for invalid in [
        r#"{"match": {"role": "row"}, "or": [{"match": {"role": "cell"}}]}"#,
        r#"{"has": {"match": {"role": "row"}}}"#,
        r#"{"match": {"role": "row"}, "has": {"hasText": "x"}}"#,
        r#"{"and": []}"#,
        r#"{"match": {"role": "row", "nth": 1}}"#,
        r#"{"match": {"role": "row"}, "visible": null}"#,
    ] {
        assert!(BrowserQuery::parse(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn release_records_are_checked() {
    let release = BrowserRelease::pinned().unwrap();
    assert!(
        release
            .platforms
            .iter()
            .any(|platform| platform.target == "aarch64-apple-darwin")
    );
    let mut record = serde_json::to_value(&release).unwrap();
    record["version"] = json!("153.0.8010");
    assert!(BrowserRelease::parse(&record.to_string()).is_err());
    record["version"] = json!("153.0.8010.36");
    record["platforms"][0]["sha256"] = json!("F".repeat(64));
    assert!(BrowserRelease::parse(&record.to_string()).is_err());
    record["platforms"][0]["sha256"] = json!("f".repeat(64));
    record["platforms"][0]["url"] = json!("not a url");
    assert!(BrowserRelease::parse(&record.to_string()).is_err());
    record["platforms"][0]["url"] = json!(release.platforms[0].url);
    // Every Linux platform, and only those, starts Chrome with the runtime's
    // libraries for its architecture.
    for platform in &release.platforms {
        assert_eq!(
            release.runtime_archives(&platform.target).is_some(),
            platform.target.contains("-linux-"),
            "{}",
            platform.target
        );
    }
    assert!(BrowserRelease::parse(&record.to_string()).is_ok());
    record["runtime"]["fonts"]["sha256"] = json!("F".repeat(64));
    assert!(BrowserRelease::parse(&record.to_string()).is_err());
}
