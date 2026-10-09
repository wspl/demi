//! `demi browser`: every operation of the package has its command, but the
//! user's Stop (`live-view.md` § The tab methods).

use demi_command_package_browser_protocol::browser::{OPERATIONS, PREFIX};
use demi_plugin_browser::Browser;
use demi_plugin_interface::{
    PluginFactory,
    testing::command_line::{help, parse, roots},
};

const TAB: &str = "t1";

#[test]
fn every_operation_has_a_command_that_takes_each_operand_from_one_source() {
    let root = roots(Browser::new().manifest()).remove(0);
    for operation in OPERATIONS {
        let name = operation.strip_prefix(PREFIX).unwrap();
        let mut line = vec!["browser"];
        line.extend(name.split('.'));
        line.push("--help");
        let parsed = parse(&root, &line, None);
        let parsed = parsed.unwrap();
        assert!(parsed.help, "{operation}");
        if name == "stop" {
            // The agent waits for its commands rather than stopping a load:
            // the help asked for is the group's.
            assert_eq!(parsed.path, ["demi", "browser"], "{operation}");
        }
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

/// A find by target flags leaves stdin to the calling process, so a job's
/// `</dev/null` or a loop's input is no query; only `--query` reads its tree
/// from stdin. Before, every find read stdin and the empty body of a
/// non-interactive job was refused as a query without `--query`.
#[test]
fn find_reads_stdin_only_with_query() {
    let root = roots(Browser::new().manifest()).remove(0);
    let find = |flags: &[&str], stdin: Option<&str>| {
        let mut line = vec!["browser", "find", TAB];
        line.extend(flags);
        parse(&root, &line, stdin)
    };
    for flags in [&["--role", "radio"][..], &["--role", "radio", "--query=false"]] {
        let parsed = find(flags, None).unwrap();
        assert!(!parsed.values.contains_key("body"), "{flags:?}");
    }
    let tree = r#"{"match":{"role":"radio"}}"#;
    assert_eq!(find(&["--query"], Some(tree)).unwrap().values["body"], tree);
    assert!(
        help(&root, &["browser", "find"]).contains("Stdin body: body, read only with --query")
    );
}

/// The `--json` results the model reads name no tab list number: only the
/// page's `user` calls answer it (`runtime.md`, "Only what the model uses").
#[test]
fn no_json_result_names_the_tab_list_number() {
    let root = roots(Browser::new().manifest()).remove(0);
    for leaf in root.leaves() {
        let schema = serde_json::to_string(leaf.json_output().expect("every leaf prints JSON")).unwrap();
        assert!(!schema.contains("\"list\""), "{}: {schema}", leaf.name);
    }
}
