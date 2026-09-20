use demi_command_service::protocol::PackageDescriptor;

#[test]
fn validates_and_hashes_typescript_package_fixture() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../packages/command-protocol/tests/fixtures/package.json"
    ))
    .unwrap();
    let descriptor = PackageDescriptor::parse(fixture["descriptor"].clone()).unwrap();
    assert_eq!(
        descriptor.digest().unwrap(),
        fixture["digest"].as_str().unwrap()
    );
    let mut invalid = fixture["descriptor"].clone();
    invalid["operations"] = serde_json::json!(["same", "same"]);
    assert!(PackageDescriptor::parse(invalid).is_err());
    // A development release carries fewer targets, but never an unknown one.
    let mut development = fixture["descriptor"].clone();
    let removed = development["targets"]
        .as_object_mut()
        .unwrap()
        .remove("aarch64-apple-darwin")
        .unwrap();
    assert!(PackageDescriptor::parse(development.clone()).is_ok());
    development["targets"]["riscv64-unknown-linux-musl"] = removed;
    assert!(PackageDescriptor::parse(development).is_err());
}

#[test]
fn generated_deserialization_enforces_package_value_constraints() {
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../packages/command-protocol/tests/fixtures/package.json"
    ))
    .unwrap();
    for (pointer, replacement) in [
        ("/id", serde_json::json!("Invalid package")),
        ("/version", serde_json::json!("")),
        ("/protocolVersion", serde_json::json!(2)),
        ("/operations", serde_json::json!([])),
        ("/operations", serde_json::json!([""])),
        ("/operations", serde_json::json!(["same", "same"])),
        (
            "/targets/aarch64-apple-darwin/sha256",
            serde_json::json!("g".repeat(64)),
        ),
        ("/targets/aarch64-apple-darwin/size", serde_json::json!(0)),
        ("/targets/aarch64-apple-darwin/size", serde_json::json!(1.5)),
        (
            "/targets/aarch64-apple-darwin/size",
            serde_json::json!(9_007_199_254_740_992_u64),
        ),
    ] {
        let mut invalid = fixture["descriptor"].clone();
        *invalid.pointer_mut(pointer).unwrap() = replacement;
        assert!(
            serde_json::from_value::<PackageDescriptor>(invalid).is_err(),
            "{pointer}"
        );
    }
}

#[test]
fn generated_invocation_checks_nested_values_and_optional_nulls() {
    use demi_command_service::protocol::{Completion, Invocation};
    let context = serde_json::json!({
        "conversation": "conversation",
        "caller": {"kind": "agent", "node": "node"},
        "locale": {"timeZone": "UTC", "languages": ["en-US"]},
    });
    let valid = serde_json::json!({"operation":"read", "invocationId":"call", "context":context, "args":{}, "cwd":"/work", "env":{}});
    assert!(serde_json::from_value::<Invocation>(valid.clone()).is_ok());
    let mut user = valid.clone();
    user["context"]["caller"] = serde_json::json!({"kind": "user"});
    assert!(serde_json::from_value::<Invocation>(user).is_ok());
    for (field, replacement) in [
        ("operation", serde_json::json!("")),
        ("invocationId", serde_json::json!("")),
        (
            "context",
            serde_json::json!({"conversation": "", "caller": context["caller"], "locale": context["locale"]}),
        ),
        (
            "context",
            serde_json::json!({"conversation": "c", "caller": {"kind": "agent"}, "locale": context["locale"]}),
        ),
        (
            "context",
            serde_json::json!({"conversation": "c", "caller": {"kind": "agent", "node": ""}, "locale": context["locale"]}),
        ),
        (
            "context",
            serde_json::json!({"conversation": "c", "caller": {"kind": "user", "node": "n"}, "locale": context["locale"]}),
        ),
        (
            "context",
            serde_json::json!({"conversation": "c", "caller": {"kind": "system"}, "locale": context["locale"]}),
        ),
        (
            "context",
            serde_json::json!({"conversation": "c", "caller": context["caller"], "locale": {"timeZone": "UTC", "languages": []}}),
        ),
        (
            "context",
            serde_json::json!({"conversation": "c", "caller": context["caller"], "locale": {"timeZone": "", "languages": ["en"]}}),
        ),
        (
            "context",
            serde_json::json!({"conversation": "c", "caller": context["caller"], "locale": {"timeZone": "UTC", "languages": vec!["en"; 17]}}),
        ),
        (
            "context",
            serde_json::json!({"conversation": "c", "caller": context["caller"]}),
        ),
        ("args", serde_json::json!([])),
        ("cwd", serde_json::json!("bad\0path")),
        ("env", serde_json::json!({"A=B":"value"})),
        ("env", serde_json::json!({"A":"bad\0value"})),
    ] {
        let mut invalid = valid.clone();
        invalid[field] = replacement;
        assert!(
            serde_json::from_value::<Invocation>(invalid).is_err(),
            "{field}"
        );
    }
    let mut missing = valid.clone();
    missing.as_object_mut().unwrap().remove("context");
    assert!(serde_json::from_value::<Invocation>(missing).is_err());
    let mut grant = valid.clone();
    grant["resource"] = serde_json::json!({"id":"old", "kind":"browser"});
    assert!(serde_json::from_value::<Invocation>(grant).is_err());
    assert!(serde_json::from_value::<Completion>(serde_json::json!({"exitCode":0})).is_ok());
    assert!(
        serde_json::from_value::<Completion>(serde_json::json!({"exitCode":0, "error":null}))
            .is_err()
    );
}
