//! What the plugin does for its commands and its page.

use demi_command_package_browser_protocol::browser::DrawResult;
use demi_command_protocol::{MAX_MEDIUM_BYTES, MediumFacts};
use demi_host_interface::{HostFile, HostRead, RpcInvocation, RpcPort};
use demi_plugin_interface::{CallKind, PluginError, PluginPort, Scope};
use schemars::JsonSchema;
use serde::Serialize;
use serde_json::{Map, Value, json};

use crate::commands::{CreateArgs, GuideArgs, Kind, draw};
use crate::frame::{self, FrameSize};
use crate::page::{self, DEFAULT_LOOK};
use crate::store;

const CREATE: &str = "demi widget create";

/// The width a widget's page has in a reply, which its screenshots are drawn
/// at: the reply's text, 800px (`packages/web-ui/src/agent/blocks/AssistantBlock.vue`),
/// less the 1px line of the card the block draws around it on each side
/// (`crates/plugin-widget/page/src/WidgetBlock.vue`).
const PAGE_WIDTH: u32 = 798;

/// How long a screenshot may take, a browser that starts for it included.
const DRAW_TIMEOUT_MS: u64 = 60_000;

/// The guide `demi widget guide` prints.
const GUIDE: &str = include_str!("../guide/guide.md");

/// Runs `create`: keeps the page, prints the widget's ID and frame, and
/// shows the agent the widget light and dark.
pub(crate) async fn create(invocation: &RpcInvocation, port: &PluginPort) -> Result<u8, PluginError> {
    let args: CreateArgs = decode(invocation)?;
    let rpc = port.rpc();
    if !args.page.trim_start().starts_with('<') {
        rpc.stderr(format!("{CREATE}: the page is not HTML: it starts with no tag\n"))
            .await?;
        return Ok(1);
    }
    let frame = match frame::declared(&args.page) {
        Ok(frame) => frame,
        Err(why) => {
            rpc.stderr(format!("{CREATE}: {why}\n")).await?;
            return Ok(1);
        }
    };
    let names = args.page.clone();
    let widget = store::add(
        port,
        &invocation.context.conversation,
        args.page.clone(),
        |number| page::file_name(&names, number),
        frame.clone(),
    )
    .await?;
    // The pages learn its frame before a reply names it.
    port.changed(Scope::User).await?;
    rpc.stdout(format!("w{}  {}\n", widget.number, frame.describe()))
        .await?;
    screenshots(port, &rpc, widget.number, &args.page, &frame).await?;
    Ok(0)
}

/// Hands the call's job the widget drawn light and dark; a Host whose
/// browser cannot draw it says so on stderr instead.
async fn screenshots(
    port: &PluginPort,
    rpc: &RpcPort,
    number: u32,
    page: &str,
    frame: &FrameSize,
) -> Result<(), PluginError> {
    for theme in ["light", "dark"] {
        let mut attributes: Map<String, Value> = DEFAULT_LOOK
            .iter()
            .map(|(name, value)| ((*name).to_owned(), json!(value)))
            .collect();
        attributes.insert("data-theme".into(), json!(theme));
        let look = json!({ "attributes": attributes });
        // The reply's background, which the frame itself does not paint.
        let prelude = format!(
            "<script>window.name = {};</script><style>html:root {{ background: var(--surface); }}</style>",
            serde_json::to_string(&look.to_string()).map_err(PluginError::failed)?
        );
        let mut args = Map::new();
        args.insert("width".into(), json!(PAGE_WIDTH));
        if let Some(height) = frame.height_at(PAGE_WIDTH) {
            args.insert("height".into(), json!(height));
        }
        args.insert("timeout".into(), json!(DRAW_TIMEOUT_MS));
        let document = page::bootstrapped(page, &prelude);
        let drawn = match port.package_call_reading(draw(), args, document, CallKind::Starts).await {
            Ok(drawn) => drawn,
            Err(error) => {
                rpc.stderr(format!("{CREATE}: screenshots skipped: {error}\n"))
                    .await?;
                return Ok(());
            }
        };
        let drawn: DrawResult = serde_json::from_value(drawn)
            .map_err(|error| PluginError::failed(format!("the browser's picture cannot be read: {error}")))?;
        // Content taller than the frame grows it once the page loads, which
        // moves the reply below it; a screenshot shows only a page that ends
        // too soon.
        if theme == "light"
            && frame.height_at(PAGE_WIDTH).is_some()
            && drawn.content_height > drawn.height
        {
            rpc.stderr(format!(
                "{CREATE}: the page's content is {content}px tall at {PAGE_WIDTH}px wide, {over}px more than its frame's {height}px: {what}\n",
                content = drawn.content_height,
                over = drawn.content_height - drawn.height,
                height = drawn.height,
                what = if frame.height.is_some() && frame.aspect_ratio.is_none() {
                    format!("the frame grows to hold it once the page loads, which moves the reply below it; declare at least {}px", drawn.content_height)
                } else {
                    "a frame of an aspect ratio keeps it, so the rest scrolls inside it; size the page by the frame".to_owned()
                },
            ))
            .await?;
        }
        // The picture is on the conversation's primary Host, which a job on
        // an attached Host does not read: its screenshots are skipped.
        let read = port
            .read_host_files(vec![HostRead::content(drawn.path.clone(), MAX_MEDIUM_BYTES)])
            .await;
        let Ok(Some(HostFile::File { bytes, size })) = read.map(|files| files.into_iter().next()) else {
            rpc.stderr(format!(
                "{CREATE}: screenshots skipped: the picture the conversation's primary Host drew cannot be read from this Host\n"
            ))
            .await?;
            return Ok(());
        };
        if size > MAX_MEDIUM_BYTES {
            rpc.stderr(format!("{CREATE}: the {theme} screenshot is over {MAX_MEDIUM_BYTES} bytes and was left out\n"))
                .await?;
            continue;
        }
        let bytes = bytes.into_bytes();
        let facts = MediumFacts {
            media_type: "image/png".into(),
            name: Some(format!("w{number}-{theme}.png")),
            width: Some(drawn.width * 2),
            height: Some(drawn.height * 2),
            duration_ms: None,
        };
        rpc.medium(facts, bytes).await?;
    }
    Ok(())
}

/// Runs `guide`: the guide, or the page of the kind it names.
pub(crate) async fn guide(invocation: &RpcInvocation, port: &PluginPort) -> Result<u8, PluginError> {
    let args: GuideArgs = decode(invocation)?;
    let text = match args.kind {
        None => GUIDE,
        Some(Kind::Line) => include_str!("../guide/kinds/line.html"),
        Some(Kind::Bar) => include_str!("../guide/kinds/bar.html"),
        Some(Kind::Scatter) => include_str!("../guide/kinds/scatter.html"),
        Some(Kind::Heatmap) => include_str!("../guide/kinds/heatmap.html"),
        Some(Kind::Share) => include_str!("../guide/kinds/share.html"),
        Some(Kind::Table) => include_str!("../guide/kinds/table.html"),
        Some(Kind::Flow) => include_str!("../guide/kinds/flow.html"),
    };
    port.rpc().stdout(text).await?;
    Ok(0)
}

/// A widget's facts, which a reply's data carries for its block: its frame
/// and its file's name.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WidgetFacts {
    pub(crate) frame: FrameSize,
    pub(crate) name: String,
}

/// The facts of the widgets `ids`, null for one the user does not have, or
/// whose value cannot be read: one such widget leaves the others' facts.
pub(crate) async fn facts(ids: Vec<String>, port: &PluginPort) -> Result<Map<String, Value>, PluginError> {
    let mut facts = Map::new();
    for id in ids {
        let widget = store::read_facts(port, &id).await.ok().flatten().map(|widget| WidgetFacts {
            frame: widget.frame,
            name: widget.name,
        });
        facts.insert(id, serde_json::to_value(widget).map_err(PluginError::failed)?);
    }
    Ok(facts)
}

/// The widget `id`'s page with its bootstrap, which the backend serves for
/// the block's frame; none for a widget the user does not have.
pub(crate) async fn document(id: &str, port: &PluginPort) -> Result<Option<String>, PluginError> {
    Ok(store::read(port, id).await?.map(|(_, page)| page::bootstrapped(&page, "")))
}

fn decode<T: serde::de::DeserializeOwned>(invocation: &RpcInvocation) -> Result<T, PluginError> {
    serde_json::from_value(Value::Object(invocation.args.clone()))
        .map_err(|error| PluginError::failed(format!("the arguments do not decode as declared: {error}")))
}

/// The guide's pages the gallery's replies show, as the reply draws them, by
/// the IDs the gallery gives them (`packages/web-gallery/src/fixtures/widgets.ts`).
pub fn specimens() -> Vec<(String, Value)> {
    [
        include_str!("../guide/kinds/line.html"),
        include_str!("../guide/kinds/heatmap.html"),
        include_str!("../guide/kinds/flow.html"),
    ]
    .iter()
    .enumerate()
    .map(|(index, kind)| {
        let number = index as u32 + 1;
        let specimen = serde_json::json!({
            "frame": frame::declared(kind).expect("a guide's page declares a frame that reads"),
            "name": page::file_name(kind, number),
            "document": page::bootstrapped(kind, ""),
        });
        (format!("w{number}"), specimen)
    })
    .collect()
}
