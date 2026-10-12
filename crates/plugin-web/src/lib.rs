//! The `web` plugin (`plugins.md` § Built-in plugins, `web.md`): the `demi
//! web` group, whose `search` runs here against Exa and whose `fetch` is
//! bound to the `demi.fetch` command package on the Host; the user's Exa
//! account, signed in through the plugin host's OAuth sign-in or as an API
//! key kept as a secret; and the Settings › Web Search page's state and
//! methods.

mod account;
mod mcp;
mod rest;
mod search;
mod settings;
mod web;

use std::rc::Rc;

use demi_plugin_interface::{
    CommandPlugin, Manifest, OAuth, Plugin, PluginError, PluginFactory, PluginId, PluginPort,
    Reply, Request,
};
use futures_util::future::LocalBoxFuture;

pub use search::{SearchArgs, SearchResults, SearchedPage};
pub use settings::{
    Account, InUse, RemoveKey, SaveKey, SavedKey, SignIn, SignInStarted, SignOut, WebReason,
    WebState,
};

/// The plugin's factory.
pub struct Web {
    manifest: Manifest,
}

impl Web {
    pub fn new() -> Self {
        let mut manifest = Manifest::new(
            PluginId::try_from("web").expect("a valid plugin id"),
            "Web",
            "Searches the web with Exa and reads pages from the conversation's Host with demi web.",
        );
        manifest.commands = web::commands().manifest_commands();
        manifest.page = Some(settings::page());
        manifest.oauth = Some(OAuth {
            vendor: "Exa".into(),
            issuer: "https://auth.exa.ai".parse().expect("a valid issuer"),
            resource: mcp::ENDPOINT.into(),
            scopes: vec!["mcp:tools".into()],
        });
        Self { manifest }
    }
}

impl Default for Web {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFactory for Web {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn instance(&self) -> Rc<dyn Plugin> {
        Rc::new(Instance(Rc::new(State {
            commands: web::commands(),
            mcp: mcp::Mcp::default(),
        })))
    }
}

/// One user's web plugin.
struct Instance(Rc<State>);

/// What the instance keeps in memory, for its user only: never a result.
struct State {
    /// The `demi web` group, whose `search` the instance answers.
    commands: CommandPlugin,
    /// The MCP sessions with Exa, anonymous and signed in.
    mcp: mcp::Mcp,
}

impl Plugin for Instance {
    fn call(
        &self,
        request: Request,
        port: PluginPort,
    ) -> LocalBoxFuture<'_, Result<Reply, PluginError>> {
        Box::pin(async move {
            match request {
                Request::PageState { .. } => {
                    let state = settings::state(&port).await?;
                    Ok(Reply::State {
                        state: serde_json::to_value(state).expect("the state encodes as JSON"),
                    })
                }
                Request::PageCall { method, params, .. } => Ok(Reply::Result {
                    result: settings::call(&method, params, &port).await?,
                }),
                Request::Command { invocation, .. } => {
                    self.0.commands.check(&invocation)?;
                    let code = search::run(&self.0.mcp, *invocation, &port).await?;
                    Ok(Reply::Exit { code })
                }
                Request::Context { .. } => Err(PluginError::undeclared("context source")),
                Request::PanelTab { .. } => Err(PluginError::undeclared("panel kind")),
                Request::PackageMessage { .. } | Request::PackageLost { .. } => {
                    Err(PluginError::undeclared("package messages"))
                }
            }
        })
    }
}
