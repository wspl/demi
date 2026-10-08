//! The `browser` plugin (`plugins.md` § Built-in plugins): the `demi
//! browser` group, every leaf bound to an operation of the `demi.browser`
//! command package, which runs it on the Host; the `browser` user stream of
//! the live view; the tab list and tab methods of the work panel's
//! `browser` kind; and the kind's tabs on the backend.

mod browser;
pub mod page;
mod panel;

use std::rc::Rc;

use demi_command_declarations::NativeOperation;
use demi_command_package_browser_protocol::{PACKAGE, live};
use demi_plugin_interface::{
    CommandPlugin, Manifest, PanelTabChange, Placement, Plugin, PluginError, PluginFactory,
    PluginId, PluginPort, Reply, Request, Scope, Stream, Topic,
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

/// The live view's user stream (`live-view.md` § The stream), with the
/// frame constants its two ends share.
fn live_stream() -> Stream {
    let operation = NativeOperation {
        package: PACKAGE.into(),
        operation: live::OPERATION.into(),
    };
    let count = |bytes: usize| u64::try_from(bytes).expect("a size fits in 64 bits");
    Stream::new::<live::LiveModuleMessage, live::LiveViewerMessage>(STREAM, operation)
        .constant(
            "LIVE_CONTROL_FRAME",
            "A control frame's kind: UTF-8 JSON of one message.",
            live::CONTROL_FRAME,
        )
        .constant(
            "LIVE_VIDEO_FRAME",
            "A video frame's kind: its header, then H.264 Annex B data.",
            live::VIDEO_FRAME,
        )
        .constant(
            "LIVE_FILE_FRAME",
            "A file frame's kind: its header, then a chosen file's bytes.",
            live::FILE_FRAME,
        )
        .constant(
            "LIVE_MAX_FRAME_BYTES",
            "The largest frame after its length.",
            count(live::MAX_FRAME_BYTES),
        )
        .constant(
            "LIVE_FILE_CHUNK_BYTES",
            "A file frame's largest data.",
            count(live::FILE_CHUNK_BYTES),
        )
        .constant(
            "LIVE_VIDEO_HEADER_BYTES",
            "A video frame's header.",
            count(live::VideoHeader::BYTES),
        )
        .constant(
            "LIVE_VIDEO_TAB_BYTES",
            "A video frame header's tab ID, padded with zero bytes.",
            count(live::VideoHeader::TAB_BYTES),
        )
        .constant(
            "LIVE_FILE_HEADER_BYTES",
            "A file frame's header.",
            count(live::FileHeader::BYTES),
        )
        .constant(
            "LIVE_HEARTBEAT_MS",
            "How often the module speaks at least.",
            live::HEARTBEAT_MS,
        )
        .constant(
            "LIVE_STALL_MS",
            "Silence after which the page shows the stream as stalled.",
            live::STALL_MS,
        )
        .constant(
            "LIVE_VIDEO_CODEC",
            "The video frames' codec, as WebCodecs names it.",
            live::VIDEO_CODEC,
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
/// the ends of jobs reach it.
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
                    conversation,
                    change,
                    tab,
                    ..
                } => {
                    match change {
                        PanelTabChange::Created => {
                            // The backend reads the tab's data, not this answer.
                            let _bound = self.work.bind(&conversation, &port, &tab.id).await?;
                        }
                        PanelTabChange::Removed => {
                            self.work.removed(&conversation, &port, &tab).await?;
                        }
                    }
                    port.changed(Scope::Conversation).await?;
                    Ok(Reply::Done)
                }
                Request::Topic {
                    topic: Topic::Jobs,
                    conversation: Some(conversation),
                    ..
                } => {
                    self.work.sync(&conversation, &port).await?;
                    Ok(Reply::Done)
                }
                Request::Topic { .. } => Err(PluginError::undeclared("topic")),
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
            }
        })
    }
}
