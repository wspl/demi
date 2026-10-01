//! What every plugin's tests run through (`plugins.md` § One contract, two
//! transports): a loopback that encodes each request, reply and port message
//! to JSON and decodes it again, so a plugin that relied on something only a
//! call in process can carry fails its tests; and a port whose rpc messages
//! go to an rpc transport, such as an in-memory port.

use std::rc::Rc;

use demi_host_interface::{PortError, PortTransport};
use futures_util::future::LocalBoxFuture;
use serde::{Serialize, de::DeserializeOwned};
use tokio_util::sync::CancellationToken;

use crate::{
    Plugin, PluginError, PluginPort, PluginTransport, PortAnswer, PortMessage, Reply, Request,
};

/// `plugin` behind the JSON loopback.
pub fn loopback(plugin: Rc<dyn Plugin>) -> Rc<dyn Plugin> {
    Rc::new(Loopback(plugin))
}

/// A port whose rpc messages `rpc` answers.
pub fn port(rpc: Rc<dyn PortTransport>) -> PluginPort {
    PluginPort::new(Rc::new(RpcAnswers(rpc)), CancellationToken::new())
}

/// The value `value` becomes after a wire: encoded to JSON and decoded.
fn across<T: Serialize + DeserializeOwned>(value: &T) -> T {
    let json = serde_json::to_value(value).expect("a plugin message encodes as JSON");
    serde_json::from_value(json).expect("a plugin message decodes from its JSON")
}

struct Loopback(Rc<dyn Plugin>);

impl Plugin for Loopback {
    fn call(
        &self,
        request: Request,
        port: PluginPort,
    ) -> LocalBoxFuture<'_, Result<Reply, PluginError>> {
        Box::pin(async move {
            let port = PluginPort::new(
                Rc::new(JsonMessages(port.transport().clone())),
                port.cancellation().clone(),
            );
            let answer = self.0.call(across(&request), port).await;
            across(&answer)
        })
    }
}

/// A port transport whose messages and answers cross the loopback.
struct JsonMessages(Rc<dyn PluginTransport>);

impl PluginTransport for JsonMessages {
    fn request(&self, message: PortMessage) -> LocalBoxFuture<'_, Result<PortAnswer, PortError>> {
        Box::pin(async move {
            let answer = self.0.request(across(&message)).await?;
            Ok(across(&answer))
        })
    }
}

struct RpcAnswers(Rc<dyn PortTransport>);

impl PluginTransport for RpcAnswers {
    fn request(&self, message: PortMessage) -> LocalBoxFuture<'_, Result<PortAnswer, PortError>> {
        Box::pin(async move {
            let PortMessage::Rpc { request } = message;
            let response = self.0.request(request).await?;
            Ok(PortAnswer::Rpc { response })
        })
    }
}

/// A plugin's command lines as a runner reads them, for the plugins' tests:
/// the plugin's trees placed as the plugin host places them, pinned to a
/// package descriptor nobody checks.
pub mod command_line {
    use std::convert::Infallible;

    use demi_command_declarations::{Group, Node, Parsed, UsageError};

    use crate::{DEMI_ROOT, DEMI_SUMMARY, Manifest, Placement};

    /// The roots `manifest`'s commands make: a `demi` root for the groups it
    /// places there, and each root of its own.
    pub fn roots(manifest: &Manifest) -> Vec<Node> {
        let mut demi = Vec::new();
        let mut roots = Vec::new();
        for commands in &manifest.commands {
            match commands.placement {
                Placement::Demi => demi.push(commands.tree.clone()),
                Placement::Root => roots.push(commands.tree.clone()),
            }
        }
        if !demi.is_empty() {
            roots.insert(
                0,
                Node::Group(Group {
                    name: DEMI_ROOT.into(),
                    summary: DEMI_SUMMARY.into(),
                    subcommands: demi,
                }),
            );
        }
        roots
            .into_iter()
            .map(|root| {
                root.pin(&mut |_| Ok::<_, Infallible>("0".repeat(64)))
                    .expect("a declaration pins")
            })
            .collect()
    }

    /// The input `<root> <line>` gives its command, with `stdin` as the body.
    pub fn parse(root: &Node, line: &[&str], stdin: Option<&str>) -> Result<Parsed, UsageError> {
        let argv = argv(line);
        let selected = root.select(&argv)?;
        let parsed = selected.parse(&argv)?;
        match selected.node.leaf() {
            Some(leaf) if !parsed.help => parsed.validate(leaf, stdin.map(str::to_owned)),
            _ => Ok(parsed),
        }
    }

    /// What `<root> <line> --help` prints.
    pub fn help(root: &Node, line: &[&str]) -> String {
        let selected = root.select(&argv(line)).expect("the line names a command");
        selected.node.help(&selected.path.join(" "))
    }

    pub fn argv(line: &[&str]) -> Vec<String> {
        line.iter().map(|word| (*word).to_owned()).collect()
    }
}
