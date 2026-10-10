//! The `demi browser` group (`browser.md`): one native leaf per operation of
//! the `demi.browser` package, its arguments and `--json` result
//! declared from the `command-package-browser-protocol` types, so the command line, the
//! runner's check and the operation read one definition.

use demi_command_declarations::NativeOperation;
use demi_command_package_browser_protocol::PACKAGE;
use demi_command_package_browser_protocol::browser::*;
use demi_host_interface::{GroupBuilder, LeafBuilder};

/// The group's summary.
const SUMMARY: &str = "Operate the conversation’s persistent browser tabs on the Host running this shell. A new Host needs demi browser install once before its first browser command. Use inspect to obtain node references; never guess them.";
/// The group's entry in the model's capability index (`system-prompt.md`
/// § Capability index).
const ENTRY: &str = "Drives a real browser on the Host: opens pages, reads their text and structure, clicks, types, fills forms, uploads and downloads files, takes screenshots, reads values with read-only scripts (demi browser eval t1 'document.title'), and keeps tabs and sign-ins for the whole conversation. Use it when a task needs a live page: checking a site the user is building, using a site that needs JavaScript or a sign-in, or showing the user a page in their work panel. Not for fetching a static URL or an API, where curl is enough.";

/// What every leaf but `screenshot` writes on success.
const SUCCESS: &str = "readable page results; one validated JSON value with --json";

const FAILURE: &str =
    "a browser error on stderr with nonzero exit status; an action is never replayed automatically";

/// When the agent shows a tab (`browser.md` § Tabs and navigation).
const SHOW: &str = "Show a tab to the user: their work panel opens and selects it once this command’s job ends. Show a tab when the user is meant to look at it or act in it: the user asked to see a page, a sign-in or a choice needs the user’s hand, or the page is a result for the user. Do not show the tabs you open to check your own work. Showing does not bring the tab to the front of the Host’s browser, and the user may have looked away again by the time you continue; showing a tab again shows it again.";

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
    "open" => OpenInput, OpenResult, "Open a URL in a new tab on this Host; starts the conversation’s browser when needed. The tab joins the user’s work panel without taking the user’s view; --show also shows it, as show does.";
    "show" => ShowInput, ShowResult, SHOW;
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
    "screenshot" => ScreenshotInput, ScreenshotResult, "Capture a tab: its PNG's bytes on stdout, so demi browser screenshot t1 | demi file view shows it to you and > shot.png saves it; or save a new PNG file with --output. Several tabs: a loop in one shell call.";
    "probe" => ProbeInput, ProbeResult, "Find elements at viewport coordinates and optionally save an annotated screenshot.";
    "click" => ClickInput, ActionResult, "Click one actionable element or an explicit viewport coordinate.";
    "move" => MoveInput, ActionResult, "Move the pointer to an element or viewport coordinate.";
    "drag" => DragInput, ActionResult, "Drag through ordered viewport points, releasing input on every exit.";
    "scroll" => ScrollInput, ActionResult, "Scroll at an element or viewport coordinate.";
    "fill" => FillInput, ActionResult, "Replace an editable element’s contents with text.";
    "type" => TypeInput, ActionResult, "Type characters into a target or the current focus, preserving selection.";
    "key" => KeyInput, ActionResult, "Press a key at the current focus, or focus a target first: key t1 Enter, key t1 e1 Enter; a key name or a + joined combination, such as Space, Enter or ControlOrMeta+A.";
    "check" => CheckInput, ActionResult, "Check a checkbox or radio; --value=false unchecks it.";
    "select" => SelectInput, ActionResult, "Select native select options by value, label or index.";
    "select-text" => SelectTextInput, ActionResult, "Select rendered text or position its cursor; prefix and suffix disambiguate.";
    "wait" => WaitInput, WaitResult, "Wait for a URL glob, element state or current-document load state, within a bounded deadline.";
    "upload" => UploadInput, UploadResult, "Attach Host files through a file input or chooser.";
    "download" => DownloadInput, DownloadResult, "Trigger and save a completed download on this Host.";
    "clipboard.write" => ClipboardWriteInput, ClipboardWriteResult, "Write finite raw stdin to the managed clipboard with the declared MIME type.";
    "clipboard.read" => ClipboardReadInput, ClipboardReadResult, "Read clipboard text or export supported MIME entries to Host files.";
    "eval" => EvalInput, EvalResult, "Run a read-only script and print the value of its last statement, as JSON: a short expression as the last argument, eval t1 'document.title', or statements from stdin. Chrome refuses side effects, and nothing waits for a Promise. Change the page with click, fill, scroll and the other actions.";
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
    "install" => InstallInput, InstallResult, "Install the pinned Chrome for Testing on this Host, on Linux with the libraries and fonts it needs.";
}

/// The `browser` group. A dotted operation name's first part is a subgroup,
/// which takes the place of its first operation.
pub(crate) fn browser_group() -> GroupBuilder {
    enum Entry {
        Leaf(Box<LeafBuilder>),
        Group(&'static str, Vec<LeafBuilder>),
    }
    let mut entries: Vec<Entry> = Vec::new();
    for (name, leaf) in leaves() {
        let Some((group, _)) = name.split_once('.') else {
            entries.push(Entry::Leaf(Box::new(leaf)));
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
        GroupBuilder::new("browser", SUMMARY).index_entry(ENTRY),
        |root, entry| match entry {
            Entry::Leaf(leaf) => root.leaf(*leaf),
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
        package: PACKAGE.into(),
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
        .positionals(positionals::<I>(name))
        .json_output::<R>()
        .success_output(match name {
            "screenshot" => {
                "the PNG's bytes on stdout and what it captured on stderr, so pipe it into demi file view to see it or redirect it to save it; with --output, the file saved and what it captured on stdout; --json requires --output"
            }
            _ => SUCCESS,
        })
        .failure_output(FAILURE);
    if let Some(field) = stdin_field(name) {
        leaf = leaf.stdin_field(field);
    }
    if name == "find" {
        // A find by target flags leaves stdin to the calling process, as a
        // job's `</dev/null` or a loop's input.
        leaf = leaf.stdin_with(["query"]);
    }
    if targeted::<I>() {
        leaf = leaf.positional_options(["ref"]);
    }
    leaf
}

/// Whether `I` takes an element target, whose reference is also the
/// positional after the tab, as outputs print it (`browser.md` § Shared
/// target grammar).
fn targeted<I: schemars::JsonSchema>() -> bool {
    schemars::schema_for!(I)
        .get("properties")
        .and_then(|properties| properties.get("ref"))
        .is_some()
}

/// The operands a command line gives in order: every operation but `open`,
/// `tabs`, `content.fetch` and `install` acts on a tab, named first; a
/// target's reference follows it, and `key`'s key comes last, after an
/// optional reference: `key t1 Enter`, `key t1 e1 Enter`; `eval`'s
/// expression, which stdin may give instead, comes last too.
fn positionals<I: schemars::JsonSchema>(name: &str) -> Vec<&'static str> {
    let mut fields = operands(name).to_vec();
    if targeted::<I>() {
        fields.push("ref");
    }
    match name {
        "key" => fields.push("key"),
        // A short expression may follow the reference instead of stdin
        // (`browser.md` § Evaluation, console, and viewport).
        "eval" => fields.push("expression"),
        _ => {}
    }
    fields
}

/// The operands before a target's reference.
fn operands(name: &str) -> &'static [&'static str] {
    match name {
        "open" => &["url"],
        "goto" => &["tab", "url"],
        "cdp.send" => &["tab", "method"],
        "webmcp.call" => &["tab", "tool"],
        "tabs" | "content.fetch" | "install" => &[],
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
