//! `demi browser`: every operation of the package has its command.

use demi_browser_protocol::browser::{OPERATIONS, PREFIX};

use crate::command_line::{demi, help, parse};

const TAB: &str = "t1";

#[test]
fn every_operation_has_a_command_that_takes_each_operand_from_one_source() {
    let (_, root) = demi();
    for operation in OPERATIONS {
        let name = operation.strip_prefix(PREFIX).unwrap();
        let mut line = vec!["browser"];
        line.extend(name.split('.'));
        line.push("--help");
        assert!(parse(&root, &line, None).unwrap().help, "{operation}");
    }
    for (line, body, field) in [
        (
            &["browser", "eval", TAB][..],
            "document.title",
            "expression",
        ),
        (
            &["browser", "find", TAB, "--query"][..],
            r#"{"match":{"role":"button"}}"#,
            "body",
        ),
        (
            &["browser", "cdp", "send", TAB, "Network.enable"][..],
            "{}",
            "params",
        ),
        (
            &[
                "browser", "webmcp", "call", TAB, "search", "--tools", "tools-1",
            ][..],
            "{}",
            "arguments",
        ),
    ] {
        let parsed = parse(&root, line, Some(body)).unwrap();
        assert_eq!(parsed.values[field], body, "{line:?}");
        let option = format!("--{field}");
        let mut given = line.to_vec();
        given.extend([option.as_str(), body]);
        assert!(parse(&root, &given, None).is_err(), "{given:?}");
    }
    let value =
        |line: &[&str], field: &str| parse(&root, line, None).unwrap().values[field].clone();
    assert_eq!(
        value(&["browser", "type", TAB, "--text", "hello"], "text"),
        "hello"
    );
    assert_eq!(value(&["browser", "cdp", "detach", TAB], "tab"), TAB);
    assert_eq!(
        value(&["browser", "key", TAB, "--key", "ControlOrMeta+A"], "key"),
        "ControlOrMeta+A"
    );
    assert_eq!(
        value(
            &[
                "browser",
                "content",
                "fetch",
                "--url",
                "https://example.test/"
            ],
            "url"
        ),
        serde_json::json!(["https://example.test/"])
    );
    assert!(help(&root, &["browser"]).contains("demi browser probe"));
}
