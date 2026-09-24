//! The `demi browser` group (`browser.md`): one native leaf per operation of
//! the `demi.builtin` package's browser, its arguments and `--json` result
//! declared from the `builtin-protocol` types, so the command line, the
//! runner's check and the operation read one definition.

use demi_builtin_protocol::browser::*;
use demi_command_tree::NativeOperation;
use demi_shell::{GroupBuilder, LeafBuilder};

use crate::BUILTIN_PACKAGE;

/// The group's summary.
const SUMMARY: &str = "Operate the conversation’s persistent browser tabs on the Host running this shell. Use inspect to obtain node references; never guess them.";

/// What every leaf but `screenshot` writes on success.
const SUCCESS: &str = "readable page results; one validated JSON value with --json";

const FAILURE: &str =
    "a browser error on stderr with nonzero exit status; an action is never replayed automatically";

/// Declares each operation's leaf from its name, its input and result types
/// and its summary; a dotted name is a leaf of a subgroup.
macro_rules! operations {
    ($($name:literal => $input:ty, $result:ty, $summary:expr;)*) => {
        /// Every browser operation's leaf, in the package's order.
        fn leaves() -> Vec<(&'static str, LeafBuilder)> {
            vec![$(($name, leaf::<$input, $result>($name, $summary)),)*]
        }
    };
}

operations! {
    "open" => OpenInput, OpenResult, "Open a URL in a new tab on this Host; starts the conversation’s browser when needed.";
    "tabs" => TabsInput, TabsResult, "List the conversation’s live browser tabs without starting a browser.";
    "info" => InfoInput, InfoResult, "Read a tab’s URL, title and viewport.";
    "goto" => GotoInput, NavigationResult, "Navigate a tab to a URL.";
    "back" => BackInput, NavigationResult, "Navigate to the previous history entry.";
    "forward" => ForwardInput, NavigationResult, "Navigate to the next history entry.";
    "reload" => ReloadInput, NavigationResult, "Reload a tab.";
    "history" => HistoryInput, HistoryResult, "Read the tab’s navigation history.";
    "close" => CloseInput, CloseResult, "Close a tab. Closing the last tab ends this browser; the next open starts fresh.";
    "inspect" => InspectInput, InspectResult, format!("Read accessibility names, roles, values, states and references; at most {MAX_NODES} nodes.");
    "find" => FindInput, FindResult, format!("Find nodes by reference, role/name, associated label, visible text, test ID or CSS; at most {MAX_NODES} nodes.");
    "read" => ReadInput, ReadResult, "Read a matched element’s text, HTML, value, attribute or visible/enabled/checked state.";
    "screenshot" => ScreenshotInput, ScreenshotResult, "Capture a tab as pure PNG stdout, or save a new PNG file with --output.";
    "probe" => ProbeInput, ProbeResult, "Find elements at viewport coordinates and optionally save an annotated screenshot.";
    "click" => ClickInput, ActionResult, "Click one actionable element or an explicit viewport coordinate.";
    "move" => MoveInput, ActionResult, "Move the pointer to an element or viewport coordinate.";
    "drag" => DragInput, ActionResult, "Drag through ordered viewport points, releasing input on every exit.";
    "scroll" => ScrollInput, ActionResult, "Scroll at an element or viewport coordinate.";
    "fill" => FillInput, ActionResult, "Replace an editable element’s contents with text.";
    "type" => TypeInput, ActionResult, "Type characters into a target or the current focus, preserving selection.";
    "key" => KeyInput, ActionResult, "Press --key at the current focus or focus a supplied target first; use a key name or a + joined combination, such as Space, Enter or ControlOrMeta+A.";
    "check" => CheckInput, ActionResult, "Set a checkbox or radio to the requested checked value.";
    "select" => SelectInput, ActionResult, "Select native select options by value, label or index.";
    "select-text" => SelectTextInput, ActionResult, "Select rendered text or position its cursor; prefix and suffix disambiguate.";
    "wait" => WaitInput, WaitResult, "Wait for a URL glob, element state or current-document load state, within a bounded deadline.";
    "upload" => UploadInput, UploadResult, "Attach Host files through a file input or chooser.";
    "download" => DownloadInput, DownloadResult, "Trigger and save a completed download on this Host.";
    "clipboard.write" => ClipboardWriteInput, ClipboardWriteResult, "Write finite raw stdin to the managed clipboard with the declared MIME type.";
    "clipboard.read" => ClipboardReadInput, ClipboardReadResult, "Read clipboard text or export supported MIME entries to Host files.";
    "eval" => EvalInput, EvalResult, "Evaluate a read-only JavaScript expression; Chrome rejects side effects.";
    "logs" => LogsInput, LogsResult, "Read console entries without clearing them; use the returned cursor to continue.";
    "viewport.set" => ViewportSetInput, ViewportResult, "Set this tab’s viewport in CSS pixels and its pixel ratio (--scale), until the user picks another mode.";
    "viewport.reset" => ViewportResetInput, ViewportResult, "Return this tab to Web mode, where the user’s live view decides its size.";
    "dialog.inspect" => DialogInspectInput, DialogInspectResult, "Read the pending JavaScript dialog.";
    "dialog.accept" => DialogAcceptInput, DialogResult, "Accept the pending JavaScript dialog, optionally supplying prompt text.";
    "dialog.dismiss" => DialogDismissInput, DialogResult, "Dismiss the pending JavaScript dialog.";
    "cdp.targets" => CdpTargetsInput, CdpTargetsResult, "List this tab and its debuggable child targets.";
    "cdp.detach" => CdpDetachInput, CdpDetachResult, "Close your debugging connection to this tab, releasing its pauses, breakpoints and interceptions.";
    "cdp.send" => CdpSendInput, CdpSendResult, "Send a scoped CDP method with a JSON parameter object from stdin.";
    "cdp.events" => CdpEventsInput, CdpEventsResult, "Read buffered CDP events or wait for events after a cursor.";
    "content.read" => ContentReadInput, ContentReadResult, "Read or save the page’s text or HTML content.";
    "content.fetch" => ContentFetchInput, ContentFetchResult, "Read up to ten URLs in temporary tabs sharing this browser’s login state.";
    "assets.list" => AssetsListInput, AssetsListResult, "Inventory observed page resources and inline SVGs.";
    "assets.export" => AssetsExportInput, AssetsExportResult, "Export selected assets from a current inventory to Host files.";
    "capabilities" => CapabilitiesInput, CapabilitiesResult, "Report the browser’s available observation and evaluation capabilities.";
    "webmcp.list" => WebmcpListInput, WebmcpListResult, "List tools registered by this page and their input schemas.";
    "webmcp.call" => WebmcpCallInput, WebmcpCallResult, "Call a registered page tool with JSON arguments from stdin.";
}

/// The `browser` group. A dotted operation name's first part is a subgroup,
/// which takes the place of its first operation.
pub(crate) fn browser_group() -> GroupBuilder {
    enum Entry {
        Leaf(LeafBuilder),
        Group(&'static str, Vec<LeafBuilder>),
    }
    let mut entries: Vec<Entry> = Vec::new();
    for (name, leaf) in leaves() {
        let Some((group, _)) = name.split_once('.') else {
            entries.push(Entry::Leaf(leaf));
            continue;
        };
        let known = entries.iter_mut().find_map(|entry| match entry {
            Entry::Group(name, members) if *name == group => Some(members),
            _ => None,
        });
        match known {
            Some(members) => members.push(leaf),
            None => entries.push(Entry::Group(group, vec![leaf])),
        }
    }
    entries.into_iter().fold(
        GroupBuilder::new("browser", SUMMARY),
        |root, entry| match entry {
            Entry::Leaf(leaf) => root.leaf(leaf),
            Entry::Group(name, members) => root.group(members.into_iter().fold(
                GroupBuilder::new(name, format!("Browser {name} operations.")),
                GroupBuilder::leaf,
            )),
        },
    )
}

/// One operation's leaf: its binding, its input with the deadline its
/// operation defaults to, the operand it takes as positionals or from stdin,
/// and its `--json` result.
fn leaf<I: BrowserInput + schemars::JsonSchema, R: schemars::JsonSchema>(
    name: &'static str,
    summary: impl Into<String>,
) -> LeafBuilder {
    let command = name.rsplit('.').next().expect("a split yields a part");
    let operation = NativeOperation {
        package: BUILTIN_PACKAGE.into(),
        operation: format!("{PREFIX}{name}"),
    };
    let mut leaf = LeafBuilder::native(command, summary, operation)
        .input::<I>()
        .describe(
            "timeout",
            format!(
                "Whole operation deadline in milliseconds; default {}, maximum {MAX_TIMEOUT_MS}.",
                I::DEFAULT_TIMEOUT_MS
            ),
        )
        .positionals(positionals(name).iter().copied())
        .json_output::<R>()
        .success_output(match name {
            "screenshot" => {
                "pure PNG bytes, or file metadata with --output; --json requires --output"
            }
            _ => SUCCESS,
        })
        .failure_output(FAILURE);
    if let Some(field) = stdin_field(name) {
        leaf = leaf.stdin_field(field);
    }
    leaf
}

/// The operands a command line gives in order: every operation but `open`,
/// `tabs` and `content.fetch` acts on a tab, named first.
fn positionals(name: &str) -> &'static [&'static str] {
    match name {
        "open" => &["url"],
        "goto" => &["tab", "url"],
        "cdp.send" => &["tab", "method"],
        "webmcp.call" => &["tab", "tool"],
        "tabs" | "content.fetch" => &[],
        _ => &["tab"],
    }
}

/// The operand an operation reads from finite standard input.
fn stdin_field(name: &str) -> Option<&'static str> {
    match name {
        "eval" => Some("expression"),
        "find" => Some("body"),
        "cdp.send" => Some("params"),
        "webmcp.call" => Some("arguments"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use demi_builtin_protocol::browser::{OPERATIONS, PREFIX};

    use crate::command_line::{demi, help, parse};

    const TAB: &str = "t_aaaaaaaaaaaaaaaaaaaaaa";

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
}
