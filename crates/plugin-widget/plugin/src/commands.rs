//! The `demi widget` group, whose leaves the plugin answers itself with the
//! request's whole port.

use demi_command_declarations::NativeOperation;
use demi_command_package_browser_protocol::PACKAGE;
use demi_host_interface::{GroupBuilder, LeafBuilder};
use demi_plugin_interface::{CommandPlugin, Commands, Placement, PortHandled};
use schemars::JsonSchema;
use serde::Deserialize;

const SUMMARY: &str = "Make an HTML page a widget, shown in place in a reply, and read how to make one.";

/// The group's entry in the model's capability index.
const ENTRY: &str = "Charts, diagrams and interactive pages in your reply: data with a shape, such as a trend, a comparison or a distribution, is shown, not told.";

const DETAILS: &str = "Read demi widget guide before the first widget of a conversation.";

const CREATE_SUMMARY: &str = "Make the HTML page on stdin a widget, and show it to yourself as the reply will, light and dark. Place it in the reply as <widget id=\"w1\" /> on a line of its own. A changed page is created again, as a widget of its own.";

const GUIDE_SUMMARY: &str = "How to make a widget: the theme its page is drawn in, its frame, its style, and a page to copy for each kind of chart and diagram.";

const PAGE: &str = "The page, a whole HTML document of at most 1 MiB, whose demi-frame meta tag, when it has one, declares its frame as demi widget guide shows; - or none reads it from stdin.";

const CREATE_OUTPUT: &str = "the widget's ID and frame, such as `w1  height 330px`, and two screenshots of it, light and dark, shown to you with the result; without the Host's browser, a line on stderr that the screenshots were skipped and how to install it";

const CREATE_FAILURE: &str = "an error on stderr that names the cause, with exit status 1; no widget is made";

/// The input of `create`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct CreateArgs {
    pub(crate) page: String,
}

/// The input of `guide`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct GuideArgs {
    pub(crate) kind: Option<Kind>,
}

/// The kinds of page the guide has one of to copy.
#[derive(Clone, Copy, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Kind {
    Line,
    Bar,
    Scatter,
    Heatmap,
    Share,
    Table,
    Flow,
}

const KIND: &str = "A kind of page to print, to copy and change; without it, the guide itself.";

/// The browser operation `create` draws its screenshots with.
pub(crate) fn draw() -> NativeOperation {
    NativeOperation {
        package: PACKAGE.into(),
        operation: "browser.draw".into(),
    }
}

/// The plugin's commands: the `demi widget` group.
pub(crate) fn commands() -> CommandPlugin {
    CommandPlugin::new(Placement::Demi, vec![widget_group()])
        .expect("the widget group is a valid declaration")
}

/// The manifest's commands, with the operation `create` calls.
pub(crate) fn manifest_commands(commands: &CommandPlugin) -> Vec<Commands> {
    let mut trees = commands.manifest_commands();
    for tree in &mut trees {
        tree.calls.push(draw());
    }
    trees
}

fn widget_group() -> GroupBuilder {
    GroupBuilder::new("widget", SUMMARY)
        .index_entry(ENTRY)
        .details(DETAILS)
        .leaf(
            LeafBuilder::rpc("create", CREATE_SUMMARY)
                .input::<CreateArgs>()
                .describe("page", PAGE)
                .positionals(["page"])
                .stdin_field("page")
                .media()
                .output(CREATE_OUTPUT)
                .failure(CREATE_FAILURE)
                .bind(PortHandled),
        )
        .leaf(
            LeafBuilder::rpc("guide", GUIDE_SUMMARY)
                .input::<GuideArgs>()
                .describe("kind", KIND)
                .positionals(["kind"])
                .output("the guide, as Markdown, or the kind's page, as HTML")
                .bind(PortHandled),
        )
}
