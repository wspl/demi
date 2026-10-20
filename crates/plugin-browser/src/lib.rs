//! The `browser` plugin: the `demi browser` group, every leaf bound to an
//! operation of the `demi.browser` command package, which runs it on the Host;
//! the `browser` user stream of the live view; the tab list and tab methods of
//! the work panel's `browser` kind; and the kind's tabs on the backend.
//!
//! It holds no browser logic: commands are declared from the operation types of
//! `command-package-browser-protocol`, the tab methods and the panel's tabs
//! call the browser's own operations as package calls, and the user stream only
//! names `browser.live`. A backend whose native catalog serves no browser has
//! no `browser` plugin methods, so the calls answer `unknown_plugin_method`.

mod browser;
pub mod page;
mod panel;
pub mod reason;

use std::rc::Rc;

use demi_command_declarations::NativeOperation;
use demi_command_package_browser_protocol::browser::{BrowserMessage, TabsChange};
use demi_command_package_browser_protocol::{PACKAGE, live};
use demi_plugin_interface::{
    CommandPlugin, Manifest, Placement, Plugin, PluginError, PluginFactory,
    PluginId, PluginPort, Reply, Request, Scope, Stream,
};
use futures_util::future::LocalBoxFuture;

/// The name of the live view's user stream.
pub const STREAM: &str = "browser";

/// The plugin's factory.
pub struct Browser {
    manifest: Manifest,
}

impl Browser {
    pub fn new() -> Self {
        let mut manifest = Manifest::new(
            PluginId::try_from("browser").expect("a valid plugin id"),
            "Browser",
            "Gives the agent a browser on the conversation's host, which it drives with demi browser and you watch and use in the work panel.",
        );
        manifest.commands = commands().manifest_commands();
        manifest.streams = vec![live_stream()];
        manifest.page = Some(page::page());
        Self { manifest }
    }
}

impl Default for Browser {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for Browser {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn instance(&self) -> Rc<dyn Plugin> {
        Rc::new(Instance {
            commands: commands(),
            work: panel::Work::default(),
        })
    }
}

/// The live view's user stream, with the
/// frame constants its two ends share.
fn live_stream() -> Stream {
    let operation = NativeOperation {
        package: PACKAGE.into(),
        operation: live::OPERATION.into(),
    };
    let count = |bytes: usize| u64::try_from(bytes).expect("a size fits in 64 bits");
    Stream::new::<live::LiveModuleMessage, live::LiveViewerMessage>(STREAM, operation)
        .constant(
            "LIVE_FILE_FRAME",
            "A file frame's kind: its header, then a chosen file's bytes.",
            live::FILE_FRAME,
        )
        .constant(
            "LIVE_FILE_CHUNK_BYTES",
            "A file frame's largest data.",
            count(live::FILE_CHUNK_BYTES),
        )
        .constant(
            "LIVE_FILE_HEADER_BYTES",
            "A file frame's header.",
            count(live::FileHeader::BYTES),
        )
        .constant(
            "LIVE_CAPTURE_UNAVAILABLE",
            "A notice's code when the Host cannot capture the watched tab.",
            live::CAPTURE_UNAVAILABLE,
        )
        .constant(
            "LIVE_CAPTURE_FAILED",
            "A notice's code when the watched tab's capture failed.",
            live::CAPTURE_FAILED,
        )
        .constant(
            "LIVE_CAPTURE_STOPPED",
            "A notice's code when the Host stopped trying to capture until the viewer asks again.",
            live::CAPTURE_STOPPED,
        )
}

/// The plugin's `demi browser` group.
fn commands() -> CommandPlugin {
    CommandPlugin::new(Placement::Demi, vec![browser::browser_group()])
        .expect("the browser group is a valid declaration")
}

/// One user's browser plugin: its commands are all native, so only its
/// conversation state, the tab list, the tab methods, its kind's tabs and
/// the browser package's messages reach it.
struct Instance {
    commands: CommandPlugin,
    work: panel::Work,
}

impl Plugin for Instance {
    fn call(
        &self,
        request: Request,
        port: PluginPort,
    ) -> LocalBoxFuture<'_, Result<Reply, PluginError>> {
        Box::pin(async move {
            match request {
                Request::Command { invocation, .. } => {
                    self.commands.command(*invocation, &port).await
                }
                Request::PageCall {
                    method,
                    params,
                    conversation: Some(conversation),
                    ..
                } => {
                    let result =
                        page::call(&method, params, &port, &self.work, &conversation).await?;
                    Ok(Reply::Result { result })
                }
                Request::PageCall {
                    conversation: None, ..
                } => Err(PluginError::undeclared("user method")),
                Request::PanelTab {
                    conversation, tab, ..
                } => {
                    self.work.removed(&conversation, &port, &tab).await?;
                    port.changed(Scope::Conversation).await?;
                    Ok(Reply::Done)
                }
                Request::PackageMessage {
                    conversation,
                    message,
                    ..
                } => {
                    let BrowserMessage {
                        tabs: TabsChange::Changed,
                    } = serde_json::from_value(message).map_err(|error| {
                        PluginError::failed(format!("the browser's message does not decode: {error}"))
                    })?;
                    self.work.sync(&conversation, &port).await?;
                    port.changed(Scope::Conversation).await?;
                    Ok(Reply::Done)
                }
                Request::PackageLost { conversation, .. } => {
                    self.work.lost(&conversation, &port).await?;
                    port.changed(Scope::Conversation).await?;
                    Ok(Reply::Done)
                }
                Request::PageState {
                    conversation: Some(_),
                    ..
                } => Ok(Reply::State {
                    state: page::tabs(&port).await?,
                }),
                Request::PageState {
                    conversation: None, ..
                } => Err(PluginError::undeclared("user state")),
                Request::Context { .. } => Err(PluginError::undeclared("context source")),
                Request::ProjectSearch { .. } => Err(PluginError::undeclared("project search")),
            }
        })
    }
}
