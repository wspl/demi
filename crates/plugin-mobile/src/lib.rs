//! The `mobile` plugin (`plugins.md` § Built-in plugins,
//! `crates/command-package-mobile/src/lib.rs`): the
//! `demi mobile` group, every leaf bound to an operation of the
//! `demi.mobile` command package, which runs it on the Host; the `mobile`
//! user stream of the live view; the methods behind the panel's controls,
//! each the operation of the command that does the same; and the `mobile`
//! kind's tabs on the backend. It is a plugin as the browser is: the commands,
//! the panel and the Host-side drivers are its own, and nothing in the agent,
//! the backend or the runner knows a simulator.

mod commands;
pub mod page;
mod panel;
pub mod reason;

use std::rc::Rc;

use demi_command_declarations::NativeOperation;
use demi_command_package_mobile_protocol::mobile::{DevicesChange, MobileMessage};
use demi_command_package_mobile_protocol::{PACKAGE, live};
use demi_plugin_interface::{
    CommandPlugin, Manifest, Placement, Plugin, PluginError, PluginFactory,
    PluginId, PluginPort, Reply, Request, Scope, Stream,
};
use futures_util::future::LocalBoxFuture;

pub use commands::leaf_operations;

/// The name of the live view's user stream.
pub const STREAM: &str = "mobile";

/// The plugin's factory.
pub struct Mobile {
    manifest: Manifest,
}

impl Mobile {
    pub fn new() -> Self {
        let mut manifest = Manifest::new(
            PluginId::try_from("mobile").expect("a valid plugin id"),
            "Simulators",
            "Gives the agent the iOS simulators of the conversation's host, which it drives with demi mobile and you watch and use in the work panel.",
        );
        manifest.commands = commands().manifest_commands();
        manifest.streams = vec![live_stream()];
        manifest.page = Some(page::page());
        Self { manifest }
    }
}

impl Default for Mobile {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for Mobile {
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
/// notice codes its two ends share.
fn live_stream() -> Stream {
    let operation = NativeOperation {
        package: PACKAGE.into(),
        operation: live::OPERATION.into(),
    };
    Stream::new::<live::LiveModuleMessage, live::LiveViewerMessage>(STREAM, operation)
        .constant(
            "LIVE_CAPTURE_FAILED",
            "A notice's code when the watched device's capture failed.",
            live::CAPTURE_FAILED,
        )
        .constant(
            "LIVE_CAPTURE_UNAVAILABLE",
            "A notice's code when the Host cannot capture the watched device.",
            live::CAPTURE_UNAVAILABLE,
        )
        .constant(
            "LIVE_INPUT_FAILED",
            "A notice's code when the viewer's input did not reach the device.",
            live::INPUT_FAILED,
        )
        .constant(
            "LIVE_NO_FOCUSED_FIELD",
            "A notice's code when the viewer's text found no focused field; its message is that text.",
            live::NO_FOCUSED_FIELD,
        )
}

/// The plugin's `demi mobile` group.
fn commands() -> CommandPlugin {
    CommandPlugin::new(Placement::Demi, vec![commands::mobile_group()])
        .expect("the mobile group is a valid declaration")
}

/// One user's mobile plugin: its commands are all native, so only its page
/// calls, its kind's tabs and the package's messages reach it.
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
                    conversation: Some(_),
                    ..
                } => {
                    let result = page::call(&method, params, &port).await?;
                    Ok(Reply::Result { result })
                }
                Request::PageCall {
                    conversation: None, ..
                } => Err(PluginError::undeclared("user method")),
                // The user closed a device's tab. The panel opens no
                // device of its own: the plugin adds the kind's tabs, for
                // the devices the agent opens.
                Request::PanelTab {
                    conversation, tab, ..
                } => {
                    self.work.removed(&conversation, &port, &tab).await?;
                    Ok(Reply::Done)
                }
                Request::PackageMessage {
                    conversation,
                    message,
                    ..
                } => {
                    let MobileMessage {
                        devices: DevicesChange::Changed,
                    } = serde_json::from_value(message).map_err(|error| {
                        PluginError::failed(format!("the mobile package's message does not decode: {error}"))
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
                Request::PageState { .. } => Err(PluginError::undeclared("page state")),
                Request::Context { .. } => Err(PluginError::undeclared("context source")),
                Request::ProjectSearch { .. } => Err(PluginError::undeclared("project search")),
            }
        })
    }
}
