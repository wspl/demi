use demi_command_protocol::PackageDescriptor;

#[test]
fn validates_and_hashes_typescript_package_fixture() {
    let fixture: serde_json::Value =
        serde_json::from_str(include_str!("fixtures/package.json")).unwrap();
    let descriptor = PackageDescriptor::parse(fixture["descriptor"].clone()).unwrap();
    assert_eq!(
        descriptor.digest().unwrap(),
        fixture["digest"].as_str().unwrap()
    );
    let mut invalid = fixture["descriptor"].clone();
    invalid["operations"] = serde_json::json!(["same", "same"]);
    assert!(PackageDescriptor::parse(invalid).is_err());
    let mut invalid = fixture["descriptor"].clone();
    invalid["targets"]
        .as_object_mut()
        .unwrap()
        .remove("aarch64-apple-darwin");
    assert!(PackageDescriptor::parse(invalid).is_err());
}
