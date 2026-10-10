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

/// A reference is the locator models use most and outputs print as
/// `[ref=e3]`, so it is also the positional after the tab; `key` takes its
/// key positionally, after an optional reference; `check` checks unless
/// `--value=false` (`browser.md` § Shared target grammar). Before, `click t1
/// e27` and `key t1 Escape` failed as unexpected positionals, and a flag
/// took the next token as its value, so `open --show <url>` lost its URL.
#[test]
fn a_reference_after_the_tab_is_the_element_and_key_takes_its_key_positionally() {
    let root = roots(Browser::new().manifest()).remove(0);
    let values = |line: &[&str]| {
        let mut argv = vec!["browser"];
        argv.extend(line);
        serde_json::Value::Object(parse(&root, &argv, None).unwrap().values)
    };
    for line in [&["click", TAB, "e3"][..], &["click", TAB, "--ref", "e3"]] {
        assert_eq!(values(line), serde_json::json!({"tab": TAB, "ref": "e3"}), "{line:?}");
    }
    assert_eq!(
        values(&["read", TAB, "e21", "--property", "text"]),
        serde_json::json!({"tab": TAB, "ref": "e21", "property": "text"})
    );
    assert_eq!(
        values(&["key", TAB, "Enter"]),
        serde_json::json!({"tab": TAB, "key": "Enter"})
    );
    assert_eq!(
        values(&["key", TAB, "e1", "ControlOrMeta+A"]),
        serde_json::json!({"tab": TAB, "ref": "e1", "key": "ControlOrMeta+A"})
    );
    assert_eq!(values(&["check", TAB, "e5"]), serde_json::json!({"tab": TAB, "ref": "e5"}));
    assert_eq!(
        values(&["check", TAB, "e5", "--value=false"]),
        serde_json::json!({"tab": TAB, "ref": "e5", "value": false})
    );
    assert_eq!(
        values(&["open", "--show", "https://example.test/"]),
        serde_json::json!({"url": "https://example.test/", "show": true})
    );
    assert!(
        help(&root, &["browser", "key"]).contains("  demi browser key <tab> [<ref>] <key> ")
    );
    let refused = |line: &[&str]| {
        let mut argv = vec!["browser"];
        argv.extend(line);
        parse(&root, &argv, None).unwrap_err().to_string()
    };
    assert!(
        refused(&["click", TAB, "e3", "--ref", "e4"])
            .starts_with("error: the argument '[ref]' cannot be used with '--ref <ref>'\n"),
    );
    assert!(
        refused(&["click", TAB, "--exact", "false"])
            .starts_with("error: unexpected argument 'false' found; --exact alone is true; write --exact=false for false\n")
    );
}

/// `eval` takes a short expression as its last positional, as models write
/// it, or statements from stdin; the token after the tab is the ref when it
/// reads as one (`browser.md` § Evaluation, console, and viewport). Before,
/// `eval t1 'getComputedStyle(…)'` failed as a ref that does not match
/// "^e[1-9][0-9]{0,14}$", and a model read the regular expression to learn
/// what it had done wrong.
#[test]
fn eval_takes_its_expression_as_the_last_positional_or_from_stdin() {
    let root = roots(Browser::new().manifest()).remove(0);
    let eval = |line: &[&str], stdin: &str| {
        let mut argv = vec!["browser", "eval", TAB];
        argv.extend(line);
        parse(&root, &argv, Some(stdin)).map(|parsed| serde_json::Value::Object(parsed.values))
    };
    let style = "getComputedStyle(document.documentElement).backgroundColor";
    assert_eq!(
        eval(&[style], "").unwrap(),
        serde_json::json!({"tab": TAB, "expression": style})
    );
    assert_eq!(
        eval(&["e21"], "element.value").unwrap(),
        serde_json::json!({"tab": TAB, "ref": "e21", "expression": "element.value"})
    );
    assert_eq!(
        eval(&["e21", "element.value"], "").unwrap(),
        serde_json::json!({"tab": TAB, "ref": "e21", "expression": "element.value"})
    );
    // Both forms at once: two lines, the error and the usage.
    let both = eval(&["document.title"], "document.URL").unwrap_err().to_string();
    let lines: Vec<&str> = both.lines().collect();
    assert_eq!(
        lines[0],
        "error: \"expression\" is given both as an argument and on stdin; give it once"
    );
    assert!(lines[1].starts_with("Usage: demi browser eval <tab> [<ref>] [<expression>] "), "{both}");
    assert!(lines[1].ends_with("; more with --help") && lines.len() == 2, "{both}");
    // A value that does not match a pattern is named with what the pattern
    // stands for, never the expression.
    assert!(
        eval(&["foo", "element.id"], "")
            .unwrap_err()
            .to_string()
            .starts_with("error: \"foo\" is not a ref, such as e12\n"),
    );
    assert!(
        parse(&root, &["browser", "info", "tab1"], None)
            .unwrap_err()
            .to_string()
            .starts_with("error: \"tab1\" is not a tab ID, such as t1\n"),
    );
    let help = help(&root, &["browser", "eval"]);
    assert!(help.contains("  demi browser eval <tab> [<ref>] [<expression>] "), "{help}");
    assert!(help.contains("Stdin body: expression, unless given as <expression>"), "{help}");
}
