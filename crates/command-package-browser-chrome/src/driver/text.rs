//! The default text of every browser result (`browser.md` § Default text):
//! concise readable lines rendered from the same typed result that `--json`
//! prints. Page content in them is quoted data: control characters and
//! terminal sequences are escaped, so a page cannot forge a line of the
//! result.

use demi_command_package_browser_protocol::release::{LinuxFont, LinuxLibrary};
use demi_command_protocol::ArtifactProgress;
use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::driver::{
    operation::{BrowserError, Result},
    protocol::{
        ActionResult, AssetsExportResult, AssetsListResult, BrowserCreatedBy, BrowserNode,
        BrowserOperation, BrowserTarget, BrowserTreeNode, BrowserViewport, CapabilitiesResult,
        CdpDetachResult, CdpEventsResult, CdpSendResult, CdpTargetsResult, ClipboardReadResult,
        ClipboardWriteResult, CloseResult, ContentFetchResult, ContentReadResult, Dialog,
        DialogInspectResult, DialogOutcome, DialogResult, DownloadResult, EvalResult, FindResult,
        HistoryResult, InfoResult, InspectResult, InstallResult, LogsResult, MouseButton, NavigationResult,
        NodeValue, OpenResult, ProbeResult, ReadResult, ResolvedElement, ScreenshotResult,
        SelectedOption, ShowResult, TabsResult, UploadResult, ViewportResult, WaitResult, WebmcpCallResult,
        WebmcpListResult,
    },
};

/// The text of `operation`'s result `value`, which the output bound already
/// shortened to fit.
pub(crate) fn render(operation: &BrowserOperation, value: Value) -> Result<String> {
    let text = match operation {
        BrowserOperation::Open(input) => open(&typed(value)?, input.show == Some(true)),
        BrowserOperation::Show(_) => {
            let result: ShowResult = typed(value)?;
            format!("Shown {} to the user.\n", result.tab)
        }
        BrowserOperation::Tabs(_) => tabs(&typed(value)?),
        BrowserOperation::Info(_) => info(&typed(value)?),
        BrowserOperation::Goto(_) | BrowserOperation::Back(_) | BrowserOperation::Forward(_) => {
            navigation("Navigated to", &typed(value)?)
        }
        BrowserOperation::Reload(_) => navigation("Reloaded", &typed(value)?),
        BrowserOperation::History(_) => history(&typed(value)?),
        BrowserOperation::Close(_) => {
            let result: CloseResult = typed(value)?;
            format!("Closed {}.\n", result.closed)
        }
        BrowserOperation::Inspect(_) => inspect(&typed(value)?),
        BrowserOperation::Find(_) => find(&typed(value)?),
        BrowserOperation::Read(_) => read(&typed(value)?),
        BrowserOperation::Screenshot(_) => screenshot(&typed(value)?),
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
            format!("{}\n", result.value)
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

/// Rows whose columns line up; the first row is the header.
fn table(rows: &[Vec<String>]) -> String {
    let columns = rows.first().map_or(0, Vec::len);
    let widths: Vec<usize> = (0..columns)
        .map(|column| {
            rows.iter()
                .map(|row| row[column].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    let mut text = String::new();
    for row in rows {
        let mut line = String::new();
        for (column, cell) in row.iter().enumerate() {
            if column + 1 == columns {
                line.push_str(cell);
            } else {
                let padding = widths[column] - cell.chars().count() + 2;
                line.push_str(cell);
                line.push_str(&" ".repeat(padding));
            }
        }
        text.push_str(line.trim_end());
        text.push('\n');
    }
    text
}

fn truncated(text: &mut String, truncated: bool) {
    if truncated {
        text.push_str("[truncated]\n");
    }
}

fn open(result: &OpenResult, shown: bool) -> String {
    let mut text = format!("Tab: {}\nURL: {}\n", result.tab, plain(&result.url));
    if let Some(title) = &result.title {
        text.push_str(&format!("Title: {}\n", plain(title)));
    }
    if shown {
        text.push_str("Shown to the user.\n");
    }
    text
}

fn created_by(created_by: &BrowserCreatedBy) -> String {
    match created_by {
        BrowserCreatedBy::Agent { number } => format!("agent {number}"),
        BrowserCreatedBy::Page { opener } => format!("page {opener}"),
        BrowserCreatedBy::Temporary { number } => format!("temporary {number}"),
        BrowserCreatedBy::User {} => "user".to_owned(),
    }
}

fn tabs(result: &TabsResult) -> String {
    if result.tabs.is_empty() {
        return "No tabs.\n".to_owned();
    }
    let mut rows = vec![vec![
        "Tab".to_owned(),
        "Title".to_owned(),
        "Created by".to_owned(),
        "URL".to_owned(),
    ]];
    rows.extend(result.tabs.iter().map(|tab| {
        vec![
            tab.id.to_string(),
            plain(&tab.title),
            created_by(&tab.created_by),
            plain(&tab.url),
        ]
    }));
    let mut text = table(&rows);
    truncated(&mut text, result.truncated);
    text
}

fn info(result: &InfoResult) -> String {
    let mut text = format!(
        "Tab: {} · {}\nURL: {}\nViewport: {}\n",
        result.tab,
        plain(&result.title),
        plain(&result.url),
        viewport(&result.viewport)
    );
    match &result.dialog {
        Some(shown) => text.push_str(&dialog(shown)),
        None => text.push_str("Dialog: none\n"),
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
    let rows: Vec<Vec<String>> = result
        .entries
        .iter()
        .map(|entry| {
            let marker = if entry.current { "*" } else { " " };
            vec![
                format!("{marker} {}", entry.index),
                plain(&entry.title),
                plain(&entry.url),
            ]
        })
        .collect();
    let mut text = if rows.is_empty() {
        "No history entries.\n".to_owned()
    } else {
        table(&rows)
    };
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

/// The text of a screenshot of `tab` returned as a medium (`browser.md`
/// § Images and large outputs): what it captured, as a saved screenshot's
/// text says it; the medium's line follows it.
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
        BrowserOperation::Check(input) => on(if input.value { "Checked" } else { "Unchecked" }),
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
    let mut rows = vec![vec![
        "Target".to_owned(),
        "Kind".to_owned(),
        "URL".to_owned(),
    ]];
    rows.extend(
        result
            .targets
            .iter()
            .map(|target| vec![plain(&target.id), plain(&target.kind), plain(&target.url)]),
    );
    let mut text = table(&rows);
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
            truncated: cut,
            ..
        } => {
            let mut text = format!(
                "Title: {}\nURL: {}\n\n{}\n",
                plain(title),
                plain(url),
                body(content)
            );
            truncated(&mut text, *cut);
            text
        }
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
    let mut text = pages.join("\n");
    truncated(&mut text, result.truncated);
    text
}

fn assets_list(result: &AssetsListResult) -> String {
    let mut text = format!("Inventory: {}\n", plain(&result.inventory));
    let rows: Vec<Vec<String>> = result
        .assets
        .iter()
        .map(|asset| {
            vec![
                format!(" {}", plain(&asset.id)),
                asset.kind.to_string(),
                plain(&asset.url),
            ]
        })
        .collect();
    if !rows.is_empty() {
        text.push_str(&table(&rows));
    }
    for svg in &result.inline_svgs {
        text.push_str(&format!(" {}  inline svg\n", plain(&svg.id)));
    }
    if result.assets.is_empty() && result.inline_svgs.is_empty() {
        text.push_str("No assets.\n");
    }
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
/// that it is being unpacked (`browser.md` § Installation).
pub fn install_progress(browser: &str, progress: ArtifactProgress) -> String {
    let browser = plain(browser);
    match progress {
        ArtifactProgress::Download { done, total } => format!(
            "Downloading {browser}: {} of {} MB\n",
            megabytes(done),
            megabytes(total)
        ),
        ArtifactProgress::Unpack => format!("Unpacking {browser}\n"),
    }
}

/// What a command that needs Chrome says on a Host without `browser`, whose
/// archive is `size` bytes: that installing it is the next step
/// (`browser.md` § Browser distribution).
pub fn not_installed(browser: &str, size: u64) -> String {
    format!(
        "{} is not installed on this Host yet. Install it with `demi browser install` ({} MB), then run this command again.",
        plain(browser),
        megabytes(size)
    )
}

/// `bytes` in whole megabytes of 2^20 bytes, rounded to the nearest.
fn megabytes(bytes: u64) -> u64 {
    (bytes + (1 << 19)) >> 20
}

/// Where the browser is, then, on Linux, what the Host lacks for it: the
/// Ubuntu packages that provide its libraries and fonts, and the AppArmor
/// profile its sandbox needs (`browser.md` § Installation).
fn install(result: &InstallResult) -> String {
    let mut text = format!(
        "Installed {} at {}\n",
        plain(&result.browser),
        plain(&result.path)
    );
    if !result.missing_libraries.is_empty() || !result.missing_fonts.is_empty() {
        text.push_str(&requirements(&result.missing_libraries, &result.missing_fonts));
    }
    if let Some(sandbox) = &result.sandbox_profile {
        let path = plain(&sandbox.path);
        text.push_str(
            "This Host restricts user namespaces with AppArmor, so Chrome's sandbox cannot start.\n",
        );
        // Not indented: a heredoc ends only at a line that is its word alone.
        text.push_str(&format!(
            "Allow them for this Chrome with a profile:\nsudo tee {path} > /dev/null <<'EOF'\n"
        ));
        for line in sandbox.profile.lines() {
            text.push_str(&format!("{}\n", plain(line)));
        }
        text.push_str(&format!("EOF\nsudo apparmor_parser -r {path}\n"));
    }
    text
}

/// The `libraries` and `fonts` a Host lacks for Chrome, and the command
/// that installs the Ubuntu packages that provide them: what `install` ends
/// with, and what a command that would start Chrome on a Host without its
/// libraries fails with (`browser.md` § Browser distribution).
pub fn requirements(libraries: &[LinuxLibrary], fonts: &[LinuxFont]) -> String {
    let mut text = String::new();
    if !libraries.is_empty() {
        let names: Vec<&str> = libraries
            .iter()
            .map(|library| library.name.as_str())
            .collect();
        text.push_str(&format!(
            "Chrome needs system libraries this Host lacks: {}\n",
            plain(&names.join(", "))
        ));
    }
    if !fonts.is_empty() {
        let purposes: Vec<&str> = fonts.iter().map(|font| font.purpose.as_str()).collect();
        text.push_str(&format!(
            "Recommended fonts are missing: {}\n",
            plain(&purposes.join("; "))
        ));
    }
    let mut packages: Vec<&str> = Vec::new();
    let wanted = libraries
        .iter()
        .map(|library| library.package.as_str())
        .chain(fonts.iter().map(|font| font.package.as_str()));
    for package in wanted {
        if !packages.contains(&package) {
            packages.push(package);
        }
    }
    // The Cloud image ships no package lists, so the command refreshes them
    // first (`browser.md` § Installation).
    text.push_str(&format!(
        "On Ubuntu, install them by running this command as printed:\n  sudo apt-get update && sudo apt-get install -y {}\n",
        plain(&packages.join(" "))
    ));
    text
}

fn capabilities(result: &CapabilitiesResult) -> String {
    let mut available = String::new();
    let mut unavailable = String::new();
    for capability in &result.capabilities {
        if capability.available {
            available.push_str(&format!("  {}\n", plain(&capability.id)));
        } else {
            unavailable.push_str(&format!("  {}", plain(&capability.id)));
            if let Some(reason) = &capability.reason {
                unavailable.push_str(&format!(" — {}", plain(reason)));
            }
            unavailable.push('\n');
        }
    }
    let mut text = String::new();
    if !available.is_empty() {
        text.push_str("Available:\n");
        text.push_str(&available);
    }
    if !unavailable.is_empty() {
        text.push_str("Unavailable:\n");
        text.push_str(&unavailable);
    }
    text
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn text(operation: &str, args: Value, result: Value) -> String {
        let operation = BrowserOperation::parse(operation, args).unwrap();
        render(&operation, result).unwrap()
    }

    /// `text` with each run of spaces as one: column widths are not a
    /// contract (`browser.md` § Default text).
    fn squeezed(text: &str) -> String {
        let mut squeezed = String::new();
        for character in text.chars() {
            if !(character == ' ' && squeezed.ends_with(' ')) {
                squeezed.push(character);
            }
        }
        squeezed
    }

    /// Every command answers readable lines by default, never the JSON
    /// object `--json` prints (`browser.md` § Default text): one row per
    /// result type, with the lines its reader looks for.
    #[test]
    fn every_result_renders_as_readable_lines() {
        let tab = json!({"tab": "t1"});
        let viewport =
            json!({"width": 390, "height": 844, "devicePixelRatio": 1, "mode": "custom"});
        let node = json!({"ref": "e1", "role": "button", "name": "Sign in", "depth": 0, "states": ["focused"]});
        let cases: Vec<(&str, Value, Value, Vec<&str>)> = vec![
            (
                "open",
                json!({"url": "http://localhost:3000/login"}),
                json!({"tab": "t1", "url": "http://localhost:3000/login", "title": "Sign in"}),
                vec![
                    "Tab: t1\n",
                    "URL: http://localhost:3000/login\n",
                    "Title: Sign in\n",
                ],
            ),
            (
                "open",
                json!({"url": "http://localhost:3000/login", "show": true}),
                json!({"tab": "t3", "url": "http://localhost:3000/login", "title": "Sign in"}),
                vec!["Tab: t3\n", "Title: Sign in\nShown to the user.\n"],
            ),
            (
                "show",
                tab.clone(),
                json!({"tab": "t1"}),
                vec!["Shown t1 to the user.\n"],
            ),
            (
                "tabs",
                json!({}),
                json!({"tabs": [
                    {"id": "t1", "title": "Sign in", "url": "http://localhost:3000/login", "createdBy": {"kind": "agent", "number": 0}, "loading": false, "shows": 0},
                    {"id": "t2", "title": "Admin", "url": "http://localhost:3000/admin", "createdBy": {"kind": "page", "opener": "t1"}, "loading": true, "shows": 1},
                ], "truncated": false}),
                vec![
                    "Tab  Title    Created by  URL\n",
                    "t1  Sign in  agent 0  http://localhost:3000/login\n",
                    "t2  Admin    page t1    http://localhost:3000/admin\n",
                ],
            ),
            (
                "info",
                tab.clone(),
                json!({"tab": "t1", "url": "http://localhost:3000/login", "title": "Sign in", "viewport": viewport}),
                vec![
                    "Tab: t1 · Sign in\n",
                    "Viewport: 390 × 844 CSS px, device pixel ratio 1, custom\n",
                    "Dialog: none\n",
                ],
            ),
            (
                "goto",
                json!({"tab": "t1", "url": "http://localhost:3000/products"}),
                json!({"tab": "t1", "url": "http://localhost:3000/products", "title": "Products"}),
                vec![
                    "Navigated to http://localhost:3000/products.\n",
                    "Title: Products\n",
                ],
            ),
            (
                "reload",
                tab.clone(),
                json!({"tab": "t1", "url": "http://localhost:3000/products"}),
                vec!["Reloaded http://localhost:3000/products.\n"],
            ),
            (
                "history",
                tab.clone(),
                json!({"entries": [
                    {"index": 0, "url": "http://localhost:3000/login", "title": "Sign in", "current": false},
                    {"index": 1, "url": "http://localhost:3000/products", "title": "Products", "current": true},
                ], "truncated": false}),
                vec![
                    "  0  Sign in   http://localhost:3000/login\n",
                    "* 1  Products  http://localhost:3000/products\n",
                ],
            ),
            (
                "close",
                tab.clone(),
                json!({"closed": "t1"}),
                vec!["Closed t1.\n"],
            ),
            (
                "inspect",
                tab.clone(),
                json!({"tab": "t1", "url": "http://localhost:3000/login", "title": "Sign in", "view": "accessibility", "truncated": false,
                    "tree": [{"role": "main", "children": [{"ref": "e2", "role": "textbox", "name": "Password", "states": ["protected"], "value": "x"}]}]}),
                vec![
                    "Tab: t1 · Sign in\nURL: http://localhost:3000/login\n\n",
                    "- main:\n",
                    "  - textbox \"Password\" [ref=e2] [protected] [value=\"x\"]\n",
                ],
            ),
            (
                "find",
                json!({"tab": "t1", "role": "button"}),
                json!({"matches": [node], "count": 1, "truncated": false}),
                vec!["1 match:\n  [ref=e1] button \"Sign in\" [focused]\n"],
            ),
            (
                "find",
                json!({"tab": "t1", "role": "link"}),
                json!({"matches": [], "count": 0, "truncated": false}),
                vec!["No matches.\n"],
            ),
            (
                "read",
                json!({"tab": "t1", "ref": "e3", "property": "text"}),
                json!({"value": "Phone A"}),
                vec!["Phone A\n"],
            ),
            (
                "read",
                json!({"tab": "t1", "css": ".product", "property": "text", "all": true}),
                json!({"values": ["Phone A", "Phone B"], "truncated": false}),
                vec!["[0] Phone A\n[1] Phone B\n"],
            ),
            (
                "screenshot",
                json!({"tab": "t1", "output": "/tmp/login.png"}),
                json!({"path": "/tmp/login.png", "mimeType": "image/png", "width": 390, "height": 844, "viewport": viewport}),
                vec![
                    "Screenshot saved: /tmp/login.png\n",
                    "Image: 390 × 844 px, one pixel per CSS pixel\n",
                ],
            ),
            (
                "probe",
                json!({"tab": "t1", "xy": "420,300"}),
                json!({"matches": [{"ref": "e1", "role": "button", "name": "Sign in", "depth": 0, "states": [],
                    "bounds": {"x": 360, "y": 280, "width": 120, "height": 40}}], "viewport": viewport, "truncated": false}),
                vec![
                    "[ref=e1] button \"Sign in\"\n",
                    "Bounds: x=360 y=280 width=120 height=40 CSS px\n",
                ],
            ),
            (
                "click",
                json!({"tab": "t1", "ref": "e1"}),
                json!({"operation": "click", "target": {"ref": "e1", "role": "button", "name": "Sign in"}, "result": "completed", "url": "http://localhost:3000/dashboard", "openedTabs": ["t2"]}),
                vec![
                    "Clicked button \"Sign in\" [ref=e1].\n",
                    "URL: http://localhost:3000/dashboard\n",
                    "Opened tab: t2\n",
                ],
            ),
            (
                "click",
                json!({"tab": "t1", "xy": "420,300", "count": 2}),
                json!({"operation": "click", "result": "completed"}),
                vec!["Double-clicked at 420,300.\n"],
            ),
            (
                "click",
                json!({"tab": "t1", "css": ".product"}),
                json!({"operation": "click", "target": {"ref": "e3", "role": "", "name": ""}, "result": "completed"}),
                vec!["Clicked [ref=e3].\n"],
            ),
            (
                "drag",
                json!({"tab": "t1", "point": ["100,200", "300,250"]}),
                json!({"operation": "drag", "result": true}),
                vec!["Dragged from 100,200 to 300,250.\n"],
            ),
            (
                "scroll",
                json!({"tab": "t1", "dy": 600.0}),
                json!({"operation": "scroll", "result": "completed"}),
                vec!["Scroll input delivered: dy=600.\n"],
            ),
            (
                "fill",
                json!({"tab": "t1", "ref": "e4", "text": "test@example.com"}),
                json!({"operation": "fill", "target": {"ref": "e4", "role": "textbox", "name": "Email"}, "result": "completed"}),
                vec!["Filled textbox \"Email\" [ref=e4].\n"],
            ),
            (
                "type",
                json!({"tab": "t1", "text": "hello"}),
                json!({"operation": "type", "target": {"ref": "e4", "role": "textbox", "name": "Email"}, "result": "completed"}),
                vec!["Typed into textbox \"Email\" [ref=e4].\n"],
            ),
            (
                "key",
                json!({"tab": "t1", "key": "Escape"}),
                json!({"operation": "key", "result": "completed"}),
                vec!["Pressed Escape in the document.\n"],
            ),
            (
                "check",
                json!({"tab": "t1", "ref": "e5", "value": false}),
                json!({"operation": "check", "target": {"ref": "e5", "role": "checkbox", "name": "Remember me"}, "result": "completed"}),
                vec!["Unchecked checkbox \"Remember me\" [ref=e5].\n"],
            ),
            (
                "select",
                json!({"tab": "t1", "ref": "e6", "value": ["SG", "JP"]}),
                json!({"operation": "select", "target": {"ref": "e6", "role": "combobox", "name": "Country"},
                    "result": [{"value": "SG", "label": "Singapore"}, {"value": "JP", "label": "JP"}]}),
                vec!["Selected: Singapore (SG), JP.\n"],
            ),
            (
                "select-text",
                json!({"tab": "t1", "ref": "e7", "text": "Replace", "cursor": "before"}),
                json!({"operation": "select-text", "target": {"ref": "e7", "role": "textbox", "name": ""}, "result": "before"}),
                vec!["Cursor placed before the matching text in textbox [ref=e7].\n"],
            ),
            (
                "wait",
                json!({"tab": "t1", "url": "**/dashboard"}),
                json!({"condition": "**/dashboard", "matched": true, "url": "http://localhost:3000/dashboard"}),
                vec!["URL matched: http://localhost:3000/dashboard.\n"],
            ),
            (
                "wait",
                json!({"tab": "t1", "role": "heading", "name": "Welcome back", "state": "visible"}),
                json!({"condition": "visible", "matched": true, "target": {"ref": "e50", "role": "heading", "name": "Welcome back"}}),
                vec!["Matched heading \"Welcome back\" [ref=e50]. State: visible.\n"],
            ),
            (
                "wait",
                json!({"tab": "t1", "css": ".spinner", "state": "detached"}),
                json!({"condition": "detached", "matched": true}),
                vec!["No element matches. State: detached.\n"],
            ),
            (
                "wait",
                json!({"tab": "t1", "load": "domcontentloaded"}),
                json!({"condition": "domcontentloaded", "matched": true}),
                vec!["Load state reached: domcontentloaded.\n"],
            ),
            (
                "upload",
                json!({"tab": "t1", "ref": "e9", "file": ["/tmp/avatar.png"]}),
                json!({"files": ["/tmp/avatar.png"], "attached": 1}),
                vec!["Attached 1 file through [ref=e9].\n  /tmp/avatar.png\n"],
            ),
            (
                "download",
                json!({"tab": "t1", "ref": "e10", "output": "/tmp/report.pdf"}),
                json!({"path": "/tmp/report.pdf", "suggestedFilename": "report.pdf", "bytes": 48320, "mimeType": "application/pdf"}),
                vec![
                    "Downloaded: /tmp/report.pdf\n",
                    "Suggested filename: report.pdf\n",
                    "Bytes: 48320\n",
                ],
            ),
            (
                "clipboard.write",
                tab.clone(),
                json!({"mimeType": "text/plain", "bytes": 6}),
                vec!["Clipboard written: text/plain, 6 bytes.\n"],
            ),
            (
                "clipboard.read",
                tab.clone(),
                json!({"text": "Hello"}),
                vec!["Hello\n"],
            ),
            (
                "eval",
                json!({"tab": "t1", "expression": "1"}),
                json!({"value": 12}),
                vec!["12\n"],
            ),
            (
                "logs",
                tab.clone(),
                json!({"entries": [{"sequence": 4, "level": "error", "text": "Failed to load orders", "url": "http://localhost:3000/orders", "timestamp": 0.0}],
                    "cursor": "logs_1:4", "hasMore": false, "truncated": false}),
                vec![
                    "[error] Failed to load orders\n  http://localhost:3000/orders\n",
                    "Cursor: logs_1:4\n",
                ],
            ),
            (
                "viewport.set",
                json!({"tab": "t1", "width": 390, "height": 844}),
                json!({"viewport": viewport}),
                vec!["Viewport: 390 × 844 CSS px, device pixel ratio 1, custom.\n"],
            ),
            (
                "dialog.inspect",
                tab.clone(),
                json!({"dialog": {"type": "confirm", "message": "Delete this record?"}}),
                vec!["Dialog: confirm\nMessage: \"Delete this record?\"\n"],
            ),
            (
                "dialog.inspect",
                tab.clone(),
                json!({"dialog": null}),
                vec!["No dialog.\n"],
            ),
            (
                "dialog.accept",
                tab.clone(),
                json!({"type": "confirm", "outcome": "accepted"}),
                vec!["Accepted confirm dialog.\n"],
            ),
            (
                "cdp.targets",
                tab.clone(),
                json!({"targets": [{"id": "main", "kind": "page", "url": "http://localhost:3000/orders"}], "truncated": false}),
                vec![
                    "Target  Kind  URL\n",
                    "main    page  http://localhost:3000/orders\n",
                ],
            ),
            (
                "cdp.detach",
                tab.clone(),
                json!({"detached": "t1"}),
                vec!["Detached debugging from t1.\n"],
            ),
            (
                "cdp.send",
                json!({"tab": "t1", "method": "Network.enable", "params": "{}"}),
                json!({"method": "Network.enable", "result": {}}),
                vec!["CDP Network.enable completed.\nResult: {}\n"],
            ),
            (
                "cdp.events",
                tab.clone(),
                json!({"events": [], "cursor": "cdp_1:120", "hasMore": false, "truncated": false}),
                vec![
                    "Cursor: cdp_1:120\nNo events. Use --after cdp_1:120 to read subsequent events.\n",
                ],
            ),
            (
                "content.read",
                tab.clone(),
                json!({"url": "http://localhost:3000/products", "title": "Products", "format": "text", "content": "Welcome.\nAll items.", "truncated": false}),
                vec![
                    "Title: Products\nURL: http://localhost:3000/products\n\nWelcome.\nAll items.\n",
                ],
            ),
            (
                "content.fetch",
                json!({"url": ["https://example.com/a"]}),
                json!({"pages": [{"requestedUrl": "https://example.com/a", "url": "https://example.com/a", "title": "Page A", "content": "Page A content."}], "truncated": false}),
                vec!["URL: https://example.com/a\nTitle: Page A\nPage A content.\n"],
            ),
            (
                "assets.list",
                tab.clone(),
                json!({"inventory": "assets_1", "assets": [{"id": "a1", "kind": "image", "url": "http://localhost:3000/logo.png"}], "inlineSvgs": [], "truncated": false}),
                vec![
                    "Inventory: assets_1\n",
                    " a1  image  http://localhost:3000/logo.png\n",
                ],
            ),
            (
                "assets.export",
                json!({"tab": "t1", "inventory": "assets_1", "output-dir": "/tmp/assets"}),
                json!({"directory": "/tmp/assets", "manifest": "/tmp/assets/manifest.json", "files": [{"id": "a1", "path": "/tmp/assets/logo.png", "bytes": 10, "mimeType": "image/png"}]}),
                vec!["Exported 1 asset to /tmp/assets.\nManifest: /tmp/assets/manifest.json\n"],
            ),
            (
                "capabilities",
                tab.clone(),
                json!({"capabilities": [{"id": "dom", "available": true}, {"id": "clipboard", "available": false, "reason": "shared with the Host user"}]}),
                vec![
                    "Available:\n  dom\n",
                    "Unavailable:\n  clipboard — shared with the Host user\n",
                ],
            ),
            (
                "webmcp.list",
                tab.clone(),
                json!({"tools": "tools_1", "entries": [{"name": "search", "description": "Search the store", "inputSchema": {"type": "object"}}], "truncated": false}),
                vec!["Tools: tools_1\n", "  search — Search the store\n"],
            ),
            (
                "webmcp.call",
                json!({"tab": "t1", "tool": "search", "tools": "tools_1", "arguments": "{}"}),
                json!({"name": "search", "result": {"count": 2}}),
                vec!["Called search.\nResult: {\"count\":2}\n"],
            ),
            (
                "install",
                json!({}),
                json!({
                    "browser": "Chrome for Testing 153.0.8010.36",
                    "path": "/home/demi/.demi/artifacts/a/chrome-linux64/chrome",
                    "missingLibraries": [
                        {"name": "libnss3.so", "package": "libnss3"},
                        {"name": "libnssutil3.so", "package": "libnss3"},
                        {"name": "libgbm.so.1", "package": "libgbm1"},
                    ],
                    "missingFonts": [
                        {"purpose": "color emoji", "file": "NotoColorEmoji.ttf", "package": "fonts-noto-color-emoji"},
                        {"purpose": "Chinese, Japanese and Korean text", "file": "NotoSansCJK-Regular.ttc", "package": "fonts-noto-cjk"},
                    ],
                }),
                vec![
                    "Installed Chrome for Testing 153.0.8010.36 at /home/demi/.demi/artifacts/a/chrome-linux64/chrome\n",
                    "Chrome needs system libraries this Host lacks: libnss3.so, libnssutil3.so, libgbm.so.1\n",
                    "Recommended fonts are missing: color emoji; Chinese, Japanese and Korean text\n",
                    // Each package once, whatever it provides.
                    "On Ubuntu, install them by running this command as printed:\n  sudo apt-get update && sudo apt-get install -y libnss3 libgbm1 fonts-noto-color-emoji fonts-noto-cjk\n",
                ],
            ),
            (
                "install",
                json!({}),
                json!({
                    "browser": "Chrome for Testing 153.0.8010.36",
                    "path": "/home/demi/.demi/artifacts/a/chrome-linux64/chrome",
                    "sandboxProfile": {
                        "path": "/etc/apparmor.d/demi-chrome",
                        "profile": "abi <abi/4.0>,\nprofile demi-chrome /home/demi/.demi/artifacts/*/chrome-linux64/chrome flags=(unconfined) {\n  userns,\n}\n",
                    },
                }),
                vec![
                    "This Host restricts user namespaces with AppArmor, so Chrome's sandbox cannot start.\n",
                    // The commands paste as they are: the heredoc's lines
                    // start at the margin.
                    "\nsudo tee /etc/apparmor.d/demi-chrome > /dev/null <<'EOF'\nabi <abi/4.0>,\nprofile demi-chrome /home/demi/.demi/artifacts/*/chrome-linux64/chrome flags=(unconfined) {\n  userns,\n}\nEOF\nsudo apparmor_parser -r /etc/apparmor.d/demi-chrome\n",
                ],
            ),
        ];
        for (operation, args, result, lines) in cases {
            let rendered = text(operation, args, result);
            assert!(
                !rendered.starts_with('{'),
                "{operation} printed JSON: {rendered}"
            );
            for line in lines {
                assert!(
                    squeezed(&rendered).contains(&squeezed(line)),
                    "{operation}: {line:?} missing from:\n{rendered}"
                );
            }
        }
    }

    /// A page cannot forge a line of a result, or command the terminal,
    /// through its title or its text (`browser.md` § Default text).
    #[test]
    fn page_text_stays_inside_its_value() {
        let title = "Sign in\nError: forged\r\u{1b}[31m";
        let rendered = text(
            "open",
            json!({"url": "http://localhost:3000/"}),
            json!({"tab": "t1", "url": "http://localhost:3000/", "title": title}),
        );
        assert!(
            rendered.contains("Title: Sign in\\nError: forged\\r\\u{1b}[31m\n"),
            "{rendered}"
        );
        assert!(!rendered.lines().any(|line| line.starts_with("Error:")));
        assert!(!rendered.contains('\u{1b}'));
        let content = text(
            "content.read",
            json!({"tab": "t1"}),
            json!({"url": "http://localhost:3000/", "title": "Page", "format": "text",
                "content": "First line\nSecond\u{1b}[2J line", "truncated": false}),
        );
        assert!(
            content.contains("First line\nSecond\\u{1b}[2J line\n"),
            "{content}"
        );
    }
}
