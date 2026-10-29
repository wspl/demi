//! The `widget` plugin: `demi widget`, and the widgets a reply shows in place.
//!
//! An agent writes an HTML page into `demi widget create`, which keeps it as
//! the user's next widget, `w1`, draws it with the Host's browser light and
//! dark for the agent to see, and prints its ID. The reply places it on a
//! line of its own, `<widget id="w1" />`, and the page's reply block draws it
//! there in a sandboxed frame, the size its page declares
//! (`crates/plugin-widget/page/src/index.ts`).
//!
//! ```text
//!   agent ── demi widget create ──> plugin ── keeps ──> value w1 + blob
//!                                     │ draw, light and dark
//!                                     v
//!                        browser package on the Host ──> two screenshots
//!   reply "<widget id="w1" />" ── its facts, frame and name, with the reply
//!                             └── page ── frames the widget's document
//! ```

mod commands;
mod frame;
mod markup;
mod page;
mod store;
mod widget;

use std::rc::Rc;

use demi_plugin_interface::{
    CommandPlugin, Manifest, Page, Plugin, PluginError, PluginFactory, PluginId, PluginPort,
    Reply, Request,
};

pub use frame::{AspectRatio, FrameSize};
pub use widget::specimens;

/// The plugin's factory.
pub struct Widgets {
    manifest: Manifest,
}

impl Widgets {
    pub fn new() -> Self {
        let mut manifest = Manifest::new(
            PluginId::try_from("widget").expect("a valid plugin id"),
            "Widgets",
            "Agents put charts, diagrams and interactive pages into their replies with demi widget.",
        );
        manifest.commands = commands::manifest_commands(&commands::commands());
        manifest.conversations = true;
        manifest.page = Some(
            Page::new("@demicodes/plugin-widget")
                .block::<widget::WidgetFacts>("widget", true)
                .panel_kind("widget"),
        );
        Self { manifest }
    }
}

impl Default for Widgets {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for Widgets {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn instance(&self) -> Rc<dyn Plugin> {
        Rc::new(Instance(commands::commands()))
    }
}

/// One user's widget plugin; its values keep the widgets.
struct Instance(CommandPlugin);

impl Plugin for Instance {
    fn call(
        &self,
        request: Request,
        port: PluginPort,
    ) -> futures_util::future::LocalBoxFuture<'_, Result<Reply, PluginError>> {
        Box::pin(async move {
            match request {
                Request::Command { invocation, .. } => {
                    self.0.check(&invocation)?;
                    let code = match invocation.path.last().map(String::as_str) {
                        Some("create") => widget::create(&invocation, &port).await?,
                        Some("guide") => widget::guide(&invocation, &port).await?,
                        _ => {
                            return Err(PluginError::failed(format!(
                                "no command \"{}\"",
                                invocation.path.join(" ")
                            )));
                        }
                    };
                    Ok(Reply::Exit { code })
                }
                Request::ReplyBlocks { ids, .. } => Ok(Reply::ReplyBlocks {
                    facts: widget::facts(ids, &port).await?,
                }),
                Request::ReplyBlockDocument { id, .. } => Ok(Reply::ReplyBlockDocument {
                    document: widget::document(&id, &port).await?,
                }),
                Request::PageCall { .. } => Err(PluginError::undeclared("method")),
                Request::PageState { .. } => Err(PluginError::undeclared("page state")),
                // A widget's tab holds nothing of the plugin's to release when it closes.
                Request::PanelTab { .. } => Ok(Reply::Done),
                Request::Context { .. } => Err(PluginError::undeclared("context source")),
                Request::ProjectSearch { .. } => Err(PluginError::undeclared("project search")),
                // The browser package it draws with keeps nothing of a
                // conversation's for it: what the package says is not its.
                Request::PackageMessage { .. } | Request::PackageLost { .. } => Ok(Reply::Done),
                Request::ConversationDeleted { conversation, .. } => {
                    store::deleted(&port, conversation.as_str()).await?;
                    Ok(Reply::Done)
                }
                Request::ConversationForked { conversation, from, .. } => {
                    store::forked(&port, from.as_str(), conversation.as_str()).await?;
                    Ok(Reply::Done)
                }
            }
        })
    }
}
