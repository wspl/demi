//! The default text of every browser result: concise readable lines rendered
//! from the same typed result that `--json` prints.
//!
//! Page content in them is quoted data: control characters and terminal
//! sequences are escaped, so a page cannot forge a line of the result. A
//! failure's last line is the error, because models read the end of a command's
//! output, often through `| tail -1`.

use demi_command_protocol::{ArtifactProgress, megabytes};
use demi_shared_types::output::Table;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::driver::{
    operation::{BrowserError, Result},
    protocol::{
        ActionResult, AssetsExportResult, AssetsListResult, BrowserCreatedBy, BrowserNode,
        BrowserOperation, BrowserTarget, BrowserTreeNode, BrowserViewport, CapabilitiesResult,
        CdpDetachResult, CdpEventsResult, CdpSendResult, CdpTargetsResult, ClipboardReadResult,
        ClipboardWriteResult, ContentFetchResult, ContentReadResult, Dialog,
        DialogInspectResult, DialogOutcome, DialogResult, DownloadResult, DrawResult, EvalResult, FindResult,
        HistoryResult, InfoResult, InspectResult, InstallResult, LogsResult, MouseButton, NavigationResult,
        NodeValue, ProbeResult, ReadResult, ResolvedElement, ScreenshotResult,
        SelectedOption, ShowResult, TabCloseResult, TabListResult, TabNewResult, UploadResult, ViewportResult, WaitResult, WebmcpCallResult,
        WebmcpListResult,
    },
};

/// The text of `operation`'s result `value`, whole: the shell that runs the
/// command cuts what it shows.
pub(crate) fn render(operation: &BrowserOperation, value: Value) -> Result<String> {
    let text = match operation {
        BrowserOperation::TabNew(_) => {
            // The ID alone, so a script takes it as T=$(demi browser tab new).
            let result: TabNewResult = typed(value)?;
            format!("{}\n", result.tab)
        }
        BrowserOperation::Show(_) => {
            let result: ShowResult = typed(value)?;
            format!("Shown {} to the user.\n", result.tab)
        }
        BrowserOperation::TabList(_) => tabs(&typed(value)?),
        BrowserOperation::Info(_) => info(&typed(value)?),
        BrowserOperation::Open(input) => {
            let mut text = navigation("Opened", &typed(value)?);
            if input.show == Some(true) {
                text.push_str("Shown to the user.\n");
            }
            text
        }
        BrowserOperation::Back(_) | BrowserOperation::Forward(_) => {
            navigation("Navigated to", &typed(value)?)
        }
        BrowserOperation::Reload(_) => navigation("Reloaded", &typed(value)?),
        // Only the user's page stops a load, and it reads the JSON.
        BrowserOperation::Stop(_) => "Stopped loading.\n".to_owned(),
        BrowserOperation::History(_) => history(&typed(value)?),
        BrowserOperation::TabClose(_) => {
            let result: TabCloseResult = typed(value)?;
            format!("Closed {}.\n", result.closed)
        }
        BrowserOperation::Inspect(_) => inspect(&typed(value)?),
        BrowserOperation::Find(_) => find(&typed(value)?),
        BrowserOperation::Read(_) => read(&typed(value)?),
        BrowserOperation::Screenshot(_) => screenshot(&typed(value)?),
        BrowserOperation::Record(input) => {
            demi_shared_recording_protocol::text::record(input.action, "demi browser record", &typed(value)?)
        }
        BrowserOperation::Probe(_) => probe(&typed(value)?),
        BrowserOperation::Click(_)
        | BrowserOperation::Move(_)
        | BrowserOperation::Drag(_)
        | BrowserOperation::Scroll(_)
        | BrowserOperation::Fill(_)
        | BrowserOperation::Type(_)
        | BrowserOperation::Key(_)
        | BrowserOperation::Check(_)
        | BrowserOperation::Select(_)
        | BrowserOperation::SelectText(_) => action(operation, &typed(value)?)?,
        BrowserOperation::Wait(input) => wait(input.load.is_some(), &typed(value)?),
        BrowserOperation::Upload(_) => upload(
            operation.target().as_ref().and_then(requested),
            &typed(value)?,
        ),
        BrowserOperation::Download(_) => download(&typed(value)?),
        BrowserOperation::ClipboardWrite(_) => {
            let result: ClipboardWriteResult = typed(value)?;
            format!(
                "Clipboard written: {}, {} bytes.\n",
                result.mime_type, result.bytes
            )
        }
        BrowserOperation::ClipboardRead(_) => clipboard(&typed(value)?),
        BrowserOperation::Eval(_) => {
            let result: EvalResult = typed(value)?;
            match result.value {
                Some(value) => format!("{value}\n"),
                None => "undefined\n".into(),
            }
        }
        BrowserOperation::Logs(_) => logs(&typed(value)?),
        BrowserOperation::ViewportSet(_) | BrowserOperation::ViewportReset(_) => {
            let result: ViewportResult = typed(value)?;
            format!("Viewport: {}.\n", viewport(&result.viewport))
        }
        BrowserOperation::DialogInspect(_) => {
            let result: DialogInspectResult = typed(value)?;
            match result.dialog {
                Some(dialog) => format!(
                    "Dialog: {}\nMessage: {}\n",
                    dialog.r#type,
                    quoted(&dialog.message)
                ),
                None => "No dialog.\n".to_owned(),
            }
        }
        BrowserOperation::DialogAccept(_) | BrowserOperation::DialogDismiss(_) => {
            let result: DialogResult = typed(value)?;
            let outcome = match result.outcome {
                DialogOutcome::Accepted => "Accepted",
                DialogOutcome::Dismissed => "Dismissed",
            };
            format!("{outcome} {} dialog.\n", result.r#type)
        }
        BrowserOperation::CdpTargets(_) => cdp_targets(&typed(value)?),
        BrowserOperation::CdpDetach(_) => {
            let result: CdpDetachResult = typed(value)?;
            format!("Detached debugging from {}.\n", result.detached)
        }
        BrowserOperation::CdpSend(_) => {
            let result: CdpSendResult = typed(value)?;
            format!(
                "CDP {} completed.\nResult: {}\n",
                plain(&result.method),
                result.result
            )
        }
        BrowserOperation::CdpEvents(_) => cdp_events(&typed(value)?),
        BrowserOperation::ContentRead(_) => content_read(&typed(value)?),
        BrowserOperation::ContentFetch(_) => content_fetch(&typed(value)?),
        BrowserOperation::AssetsList(_) => assets_list(&typed(value)?),
        BrowserOperation::AssetsExport(_) => assets_export(&typed(value)?),
        BrowserOperation::Capabilities(_) => capabilities(&typed(value)?),
        BrowserOperation::Install(_) => install(&typed(value)?),
        BrowserOperation::Draw(_) => {
            let result: DrawResult = typed(value)?;
            format!("Drew {}×{}\n", result.width, result.height)
        }
        BrowserOperation::WebmcpList(_) => webmcp_list(&typed(value)?),
        BrowserOperation::WebmcpCall(_) => {
            let result: WebmcpCallResult = typed(value)?;
            format!(
                "Called {}.\nResult: {}\n",
                plain(&result.name),
                result.result
            )
        }
    };
    Ok(text)
}

/// `value` as the result type its operation answers.
fn typed<T: DeserializeOwned>(value: Value) -> Result<T> {
    serde_json::from_value(value).map_err(|error| BrowserError::InvalidResult(error.to_string()))
}

/// Page data on one line: control characters and terminal sequences escaped,
/// so a value cannot start a line of its own or command the terminal.
pub(crate) fn plain(value: &str) -> String {
    escaped(value, |_| false)
}

/// A page's text, which keeps its lines and tabs; every other control
/// character is escaped.
pub(crate) fn body(value: &str) -> String {
    escaped(value, |character| matches!(character, '\n' | '\t'))
}

/// `value` with each control character but those `kept` escaped.
fn escaped(value: &str, kept: impl Fn(char) -> bool) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if character.is_control() && !kept(character) {
            escaped.extend(character.escape_debug());
        } else {
            escaped.push(character);
        }
    }
    escaped
}

/// Page data in quotes, with quotes and control characters escaped.
fn quoted(value: &str) -> String {
    format!("{value:?}")
}

fn viewport(viewport: &BrowserViewport) -> String {
    format!(
        "{} × {} CSS px, device pixel ratio {}, {}",
        viewport.width, viewport.height, viewport.device_pixel_ratio, viewport.mode
    )
}

fn dialog(dialog: &Dialog) -> String {
    format!("Dialog: {} {}\n", dialog.r#type, quoted(&dialog.message))
}

/// Rows whose columns line up under the header row `columns`, as every
/// command's lists print (`crates/shared-types/src/output.rs`).
fn table<const N: usize>(columns: [&str; N], rows: impl IntoIterator<Item = [String; N]>) -> String {
    let mut table = Table::new(columns);
    for row in rows {
        table.row(row);
    }
    table.render()
}

fn truncated(text: &mut String, truncated: bool) {
    if truncated {
        text.push_str("[truncated]\n");
    }
}

fn created_by(created_by: &BrowserCreatedBy) -> String {
    match created_by {
        BrowserCreatedBy::Agent { number } => format!("agent {number}"),
        BrowserCreatedBy::Page { opener } => format!("page {opener}"),
        BrowserCreatedBy::Temporary { number } => format!("temporary {number}"),
        BrowserCreatedBy::User {} => "user".to_owned(),
    }
}

fn tabs(result: &TabListResult) -> String {
    let mut text = table(
        ["tab", "created by", "url", "title"],
        result.tabs.iter().map(|tab| {
            [tab.id.to_string(), created_by(&tab.created_by), plain(&tab.url), plain(&tab.title)]
        }),
    );
    truncated(&mut text, result.truncated);
    text
}

fn info(result: &InfoResult) -> String {
    let mut text = format!(
        "Tab: {}\nTitle: {}\nURL: {}\nViewport: {}\n",
        result.tab,
        plain(&result.title),
        plain(&result.url),
        viewport(&result.viewport)
    );
    match &result.dialog {
        Some(shown) => text.push_str(&dialog(shown)),
        None => text.push_str("Dialog: -\n"),
    }
    text
}

fn navigation(verb: &str, result: &NavigationResult) -> String {
    let mut text = format!("{verb} {}.\n", plain(&result.url));
    if let Some(title) = &result.title {
        text.push_str(&format!("Title: {}\n", plain(title)));
    }
    text
}

fn history(result: &HistoryResult) -> String {
    let mut text = table(
        ["index", "current", "url", "title"],
        result.entries.iter().map(|entry| {
            [
                entry.index.to_string(),
                if entry.current { "yes" } else { "no" }.to_owned(),
                plain(&entry.url),
                plain(&entry.title),
            ]
        }),
    );
    truncated(&mut text, result.truncated);
    text
}

fn node_value(value: &NodeValue) -> String {
    match value {
        NodeValue::Text(text) => quoted(text),
        NodeValue::Number(number) => number.to_string(),
    }
}

/// A node as a match lists it: `[ref=e3] button "Sign in"`, then its states.
fn node(node: &BrowserNode) -> String {
    let mut line = String::new();
    if let Some(reference) = &node.r#ref {
        line.push_str(&format!("[ref={reference}] "));
    }
    line.push_str(&format!("{} {}", plain(&node.role), quoted(&node.name)));
    for state in &node.states {
        line.push_str(&format!(" [{}]", plain(state)));
    }
    if let Some(value) = &node.value {
        line.push_str(&format!(" [value={}]", node_value(value)));
    }
    line
}

/// A tree node's own line: its role or tag, name, reference, states and
/// value, with a colon when children follow.
fn tree_node(node: &BrowserTreeNode) -> String {
    let mut line = String::from("- ");
    line.push_str(&plain(
        node.role
            .as_deref()
            .or(node.tag.as_deref())
            .unwrap_or("node"),
    ));
    if let Some(name) = node.name.as_deref().filter(|name| !name.is_empty()) {
        line.push_str(&format!(" {}", quoted(name)));
    }
    if let Some(reference) = &node.r#ref {
        line.push_str(&format!(" [ref={reference}]"));
    }
    for state in node.states.iter().flatten() {
        line.push_str(&format!(" [{}]", plain(state)));
    }
    if let Some(value) = &node.value {
        line.push_str(&format!(" [value={}]", node_value(value)));
    }
    if node
        .children
        .as_ref()
        .is_some_and(|children| !children.is_empty())
    {
        line.push(':');
    }
    line
}

fn inspect(result: &InspectResult) -> String {
    let mut text = format!(
        "Tab: {} · {}\nURL: {}\n\n",
        result.tab,
        plain(&result.title),
        plain(&result.url)
    );
    let mut pending: Vec<_> = result
        .tree
        .iter()
        .rev()
        .map(|node| (node, 0_usize))
        .collect();
    while let Some((node, depth)) = pending.pop() {
        if let Some(children) = &node.children {
            pending.extend(children.iter().rev().map(|child| (child, depth + 1)));
        }
        text.push_str(&"  ".repeat(depth.min(64)));
        text.push_str(&tree_node(node));
        text.push('\n');
    }
    if result.tree.is_empty() {
        text.push_str("No nodes.\n");
    }
    truncated(&mut text, result.truncated);
    text
}

fn find(result: &FindResult) -> String {
    let mut text = match result.count {
        0 => return "No matches.\n".to_owned(),
        1 => "1 match:\n".to_owned(),
        count => format!("{count} matches:\n"),
    };
    for matched in &result.matches {
        text.push_str(&format!("  {}\n", node(matched)));
    }
    truncated(&mut text, result.truncated);
    text
}

/// A value a page gave: text as it is on one line, anything else as JSON.
fn value(value: &Value) -> String {
    match value {
        Value::String(text) => plain(text),
        other => other.to_string(),
    }
}

fn read(result: &ReadResult) -> String {
    match result {
        ReadResult::One { value: one } => format!("{}\n", value(one)),
        ReadResult::All {
            values,
            truncated: cut,
        } => {
            let mut text: String = values
                .iter()
                .enumerate()
                .map(|(index, each)| format!("[{index}] {}\n", value(each)))
                .collect();
            if values.is_empty() {
                text.push_str("No matches.\n");
            }
            truncated(&mut text, *cut);
            text
        }
    }
}

fn screenshot(result: &ScreenshotResult) -> String {
    format!(
        "Screenshot saved: {}\n{}",
        plain(&result.path),
        image(result.width, result.height, &result.viewport)
    )
}

/// What a screenshot of `tab` without `--output` captured, which it writes
/// to stderr beside its PNG's bytes, as a saved screenshot's text says it.
pub fn captured(tab: &str, width: u32, height: u32, shown: &BrowserViewport) -> String {
    format!("Screenshot of {}\n{}", plain(tab), image(width, height, shown))
}

/// A screenshot's image and the viewport it shows.
fn image(width: u32, height: u32, shown: &BrowserViewport) -> String {
    format!(
        "Image: {width} × {height} px, one pixel per CSS pixel\nViewport: {}\n",
        viewport(shown)
    )
}

fn probe(result: &ProbeResult) -> String {
    let mut text = String::new();
    for found in &result.matches {
        text.push_str(&node(found));
        text.push('\n');
        if let Some(bounds) = &found.bounds {
            text.push_str(&format!(
                "Bounds: x={} y={} width={} height={} CSS px\n",
                bounds.x, bounds.y, bounds.width, bounds.height
            ));
        }
    }
    if result.matches.is_empty() {
        text.push_str("No nodes at that point.\n");
    }
    if let Some(path) = &result.path {
        text.push_str(&format!("Annotated screenshot saved: {}\n", plain(path)));
    }
    truncated(&mut text, result.truncated);
    text
}

/// The element target an input named: its reference, or its role and name,
/// label, placeholder, text, test ID or CSS selector.
fn requested(target: &BrowserTarget) -> Option<String> {
    if let Some(reference) = &target.r#ref {
        return Some(format!("[ref={reference}]"));
    }
    if let Some(role) = &target.role {
        return Some(match &target.name {
            Some(name) => format!("{} {}", plain(role), quoted(name)),
            None => plain(role),
        });
    }
    let described = [
        ("label", &target.label),
        ("placeholder", &target.placeholder),
        ("text", &target.text_match),
        ("test ID", &target.test_id),
        ("css", &target.css),
    ];
    described.into_iter().find_map(|(kind, value)| {
        value
            .as_ref()
            .map(|value| format!("{kind} {}", quoted(value)))
    })
}

/// The element a target resolved to, as an action or a wait names it:
/// `button "Sign in" [ref=e3]`, leaving out a role or name the page tree
/// does not give it.
fn element(named: &ResolvedElement) -> String {
    let mut parts = Vec::new();
    if !named.role.is_empty() {
        parts.push(plain(&named.role));
    }
    if !named.name.is_empty() {
        parts.push(quoted(&named.name));
    }
    parts.push(format!("[ref={}]", named.r#ref));
    parts.join(" ")
}

/// What an action acted on: the element its target resolved to, or, for
/// untargeted keyboard input that found no focused element, the document.
fn acted_on(operation: &BrowserOperation, result: &ActionResult) -> Option<String> {
    match &result.target {
        Some(named) => Some(element(named)),
        None if matches!(
            operation,
            BrowserOperation::Type(_) | BrowserOperation::Key(_)
        ) =>
        {
            Some("the document".to_owned())
        }
        None => None,
    }
}

/// A selected option as `select` names it: its label and, when they
/// differ, its value, as in `Singapore (SG)`.
fn option(selected: &SelectedOption) -> String {
    if selected.label.is_empty() || selected.label == selected.value {
        plain(&selected.value)
    } else {
        format!("{} ({})", plain(&selected.label), plain(&selected.value))
    }
}

fn action(operation: &BrowserOperation, result: &ActionResult) -> Result<String> {
    let target = acted_on(operation, result);
    let on = |verb: &str| match &target {
        Some(target) => format!("{verb} {target}."),
        None => format!("{verb}."),
    };
    let mut text = match operation {
        BrowserOperation::Click(input) => {
            let verb = match (input.count, input.button) {
                (Some(2), _) => "Double-clicked",
                (_, Some(MouseButton::Right)) => "Right-clicked",
                (_, Some(MouseButton::Middle)) => "Middle-clicked",
                _ => "Clicked",
            };
            match &input.xy {
                Some(point) if target.is_none() => format!("{verb} at {}.", plain(point)),
                _ => on(verb),
            }
        }
        BrowserOperation::Move(input) => match &input.xy {
            Some(point) if target.is_none() => format!("Pointer moved to {}.", plain(point)),
            _ => on("Pointer moved to"),
        },
        BrowserOperation::Drag(input) => match (input.point.first(), input.point.last()) {
            (Some(from), Some(to)) => format!("Dragged from {} to {}.", plain(from), plain(to)),
            _ => "Dragged.".to_owned(),
        },
        BrowserOperation::Scroll(input) => {
            let mut amounts = Vec::new();
            if let Some(dx) = input.dx {
                amounts.push(format!("dx={dx}"));
            }
            if let Some(dy) = input.dy {
                amounts.push(format!("dy={dy}"));
            }
            let place = target
                .as_ref()
                .map_or(String::new(), |target| format!(" to {target}"));
            format!("Scroll input delivered{place}: {}.", amounts.join(" "))
        }
        BrowserOperation::Fill(_) => on("Filled"),
        BrowserOperation::Type(_) => match &target {
            Some(target) => format!("Typed into {target}."),
            None => "Typed.".to_owned(),
        },
        BrowserOperation::Key(input) => match &target {
            Some(target) => format!("Pressed {} in {target}.", plain(&input.key)),
            None => format!("Pressed {}.", plain(&input.key)),
        },
        BrowserOperation::Check(input) => on(if input.checked() { "Checked" } else { "Unchecked" }),
        BrowserOperation::Select(_) => {
            let selected: Vec<SelectedOption> = typed(result.result.clone())?;
            let selected: Vec<String> = selected.iter().map(option).collect();
            format!("Selected: {}.", selected.join(", "))
        }
        BrowserOperation::SelectText(_) => {
            let place = target
                .as_ref()
                .map_or(String::new(), |target| format!(" in {target}"));
            match result.result.as_str() {
                Some("before") => format!("Cursor placed before the matching text{place}."),
                Some("after") => format!("Cursor placed after the matching text{place}."),
                _ => format!("Selected text{place}."),
            }
        }
        _ => format!("{} completed.", plain(&result.operation)),
    };
    text.push('\n');
    if let Some(url) = &result.url {
        text.push_str(&format!("URL: {}\n", plain(url)));
    }
    for opened in result.opened_tabs.iter().flatten() {
        text.push_str(&format!("Opened tab: {opened}\n"));
    }
    if let Some(shown) = &result.dialog {
        text.push_str(&dialog(shown));
    }
    Ok(text)
}

/// A wait's text; `load` says it waited for the document's load.
fn wait(load: bool, result: &WaitResult) -> String {
    let condition = plain(&result.condition);
    if let Some(url) = &result.url {
        format!("URL matched: {}.\n", plain(url))
    } else if load {
        format!("Load state reached: {condition}.\n")
    } else if let Some(named) = &result.target {
        format!("Matched {}. State: {condition}.\n", element(named))
    } else {
        format!("No element matches. State: {condition}.\n")
    }
}

fn upload(through: Option<String>, result: &UploadResult) -> String {
    let files = if result.attached == 1 {
        "file"
    } else {
        "files"
    };
    let through = through.map_or(String::new(), |target| format!(" through {target}"));
    let mut text = format!("Attached {} {files}{through}.\n", result.attached);
    for file in &result.files {
        text.push_str(&format!("  {}\n", plain(file)));
    }
    text
}

fn download(result: &DownloadResult) -> String {
    let mut text = format!("Downloaded: {}\n", plain(&result.path));
    if !result.suggested_filename.is_empty() {
        text.push_str(&format!(
            "Suggested filename: {}\n",
            plain(&result.suggested_filename)
        ));
    }
    text.push_str(&format!("Bytes: {}\n", result.bytes));
    text
}

fn clipboard(result: &ClipboardReadResult) -> String {
    match result {
        ClipboardReadResult::Text { text } => format!("{}\n", body(text)),
        ClipboardReadResult::Items { items } => {
            let mut text = "Clipboard exported:\n".to_owned();
            for item in items {
                text.push_str(&format!("  {}: {}\n", item.mime_type, plain(&item.path)));
            }
            text
        }
    }
}

fn logs(result: &LogsResult) -> String {
    let mut text = String::new();
    for entry in &result.entries {
        text.push_str(&format!("[{}] {}\n", entry.level, plain(&entry.text)));
        if let Some(url) = &entry.url {
            text.push_str(&format!("  {}\n", plain(url)));
        }
    }
    if result.entries.is_empty() {
        text.push_str("No log entries.\n");
    }
    text.push_str(&format!("Cursor: {}\n", plain(&result.cursor)));
    if result.has_more {
        text.push_str("Has more: true\n");
    }
    truncated(&mut text, result.truncated);
    text
}

fn cdp_targets(result: &CdpTargetsResult) -> String {
    let mut text = table(
        ["target", "kind", "url"],
        result
            .targets
            .iter()
            .map(|target| [plain(&target.id), plain(&target.kind), plain(&target.url)]),
    );
    truncated(&mut text, result.truncated);
    text
}

fn cdp_events(result: &CdpEventsResult) -> String {
    let cursor = plain(&result.cursor);
    if result.events.is_empty() {
        return format!(
            "Cursor: {cursor}\nNo events. Use --after {cursor} to read subsequent events.\n"
        );
    }
    let mut text = String::new();
    for event in &result.events {
        text.push_str(&format!("[{}] {}\n", event.sequence, plain(&event.method)));
        match &event.params {
            Value::Object(params) => {
                for (key, param) in params {
                    text.push_str(&format!("  {}: {}\n", plain(key), value(param)));
                }
            }
            other => text.push_str(&format!("  {other}\n")),
        }
    }
    text.push_str(&format!(
        "Cursor: {cursor}\nHas more: {}\nTruncated: {}\n",
        result.has_more, result.truncated
    ));
    text
}

fn content_read(result: &ContentReadResult) -> String {
    match result {
        ContentReadResult::Inline {
            url,
            title,
            content,
            ..
        } => format!(
            "Title: {}\nURL: {}\n\n{}\n",
            plain(title),
            plain(url),
            body(content)
        ),
        ContentReadResult::File { path, format, .. } => {
            format!("Content saved: {}\nFormat: {format}\n", plain(path))
        }
    }
}

fn content_fetch(result: &ContentFetchResult) -> String {
    let mut pages = Vec::new();
    for page in &result.pages {
        let url = if page.url.is_empty() {
            &page.requested_url
        } else {
            &page.url
        };
        let mut text = format!("URL: {}\n", plain(url));
        match &page.error {
            Some(failure) => text.push_str(&format!(
                "Error: {}\n{}\n",
                failure.code,
                plain(&failure.message)
            )),
            None => text.push_str(&format!(
                "Title: {}\n{}\n",
                plain(&page.title),
                body(&page.content)
            )),
        }
        pages.push(text);
    }
    pages.join("\n")
}

fn assets_list(result: &AssetsListResult) -> String {
    let mut text = format!("Inventory: {}\n\n", plain(&result.inventory));
    let assets = result
        .assets
        .iter()
        .map(|asset| [plain(&asset.id), asset.kind.to_string(), plain(&asset.url)]);
    let svgs = result
        .inline_svgs
        .iter()
        .map(|svg| [plain(&svg.id), "inline svg".to_owned(), String::new()]);
    text.push_str(&table(["id", "kind", "url"], assets.chain(svgs)));
    truncated(&mut text, result.truncated);
    text
}

fn assets_export(result: &AssetsExportResult) -> String {
    let assets = if result.files.len() == 1 {
        "asset"
    } else {
        "assets"
    };
    format!(
        "Exported {} {assets} to {}.\nManifest: {}\n",
        result.files.len(),
        plain(&result.directory),
        plain(&result.manifest)
    )
}

/// A line of `install`'s download of `browser`: how much has arrived, or
/// that it is being unpacked.
pub fn install_progress(browser: &str, progress: ArtifactProgress) -> String {
    progress.line(&plain(browser))
}

/// What a command that needs Chrome says on a Host without `browser`, whose
/// archive is `size` bytes: that installing it is the next step.
pub fn not_installed(browser: &str, size: u64) -> String {
    format!(
        "{} is not installed on this Host yet. Install it with `demi browser install` ({} MB), then run this command again.",
        plain(browser),
        megabytes(size)
    )
}

/// Where the browser is.
fn install(result: &InstallResult) -> String {
    format!(
        "Installed {} at {}\n",
        plain(&result.browser),
        plain(&result.path)
    )
}

fn capabilities(result: &CapabilitiesResult) -> String {
    table(
        ["capability", "available", "reason"],
        result.capabilities.iter().map(|capability| {
            [
                plain(&capability.id),
                if capability.available { "yes" } else { "no" }.to_owned(),
                capability.reason.as_deref().map(plain).unwrap_or_default(),
            ]
        }),
    )
}

fn webmcp_list(result: &WebmcpListResult) -> String {
    let mut text = format!("Tools: {}\n", plain(&result.tools));
    for tool in &result.entries {
        text.push_str(&format!(
            "  {} — {}\n",
            plain(&tool.name),
            plain(&tool.description)
        ));
        text.push_str(&format!("    Input: {}\n", tool.input_schema));
    }
    if result.entries.is_empty() {
        text.push_str("No tools.\n");
    }
    truncated(&mut text, result.truncated);
    text
}
