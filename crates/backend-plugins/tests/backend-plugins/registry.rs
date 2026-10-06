//! The registry (`plugins.md` § Commands): what stops the
//! backend from starting, which trees are left out, and how a call through
//! the composed command set reaches the plugin's instance.

use std::rc::Rc;

use demi_backend_plugins::{Registry, RegistryError};
use demi_command_declarations::{NativeOperation, Node};
use demi_host_interface::{
    CommandSet, GroupBuilder, LeafBuilder, RpcError, RpcHandler, RpcInvocation, RpcPort,
    testing::{MemoryPort, test_command_context},
};
use demi_plugin_interface::{
    CommandPlugin, Commands, Manifest, Placement, Plugin, PluginError, PluginFactory, PluginId,
    PluginPort, Reply, Request,
};
use futures_util::future::LocalBoxFuture;

use crate::support::user_plugins;

/// A plugin whose every command prints the user and the path it was
/// handed.
struct Probe(Manifest);

impl Probe {
    fn new(id: &str) -> Self {
        Self(Manifest::new(
            PluginId::try_from(id).unwrap(),
            id,
            "A probe.",
        ))
    }

    /// Adds `group` with one rpc leaf `run`, placed at `placement`.
    fn rpc(mut self, placement: Placement, group: &str) -> Self {
        self.0.commands.push(tree(placement, rpc_group(group)));
        self
    }

    /// Adds `group` with one leaf that runs `operation` of `package`.
    fn native(mut self, group: &str, package: &str) -> Self {
        let leaf = LeafBuilder::native(
            "run",
            "Run.",
            NativeOperation {
                package: package.into(),
                operation: "run".into(),
            },
        );
        let group = GroupBuilder::new(group, "A native group.").leaf(leaf);
        self.0.commands.push(tree(Placement::Demi, group));
        self
    }

    fn boxed(self) -> Box<dyn PluginFactory> {
        Box::new(self)
    }
}

impl PluginFactory for Probe {
    fn manifest(&self) -> &Manifest {
        &self.0
    }

    fn instance(&self) -> Rc<dyn Plugin> {
        Rc::new(Printer)
    }
}

struct Printer;

impl Plugin for Printer {
    fn call(
        &self,
        request: Request,
        port: PluginPort,
    ) -> LocalBoxFuture<'_, Result<Reply, PluginError>> {
        Box::pin(async move {
            let Request::Command { user, invocation } = request else {
                unreachable!("the probe declares no page")
            };
            let line = format!("{user}: {}", invocation.path.join(" "));
            port.rpc()
                .stdout(line)
                .await
                .map_err(|error| PluginError::from(RpcError::Port(error)))?;
            Ok(Reply::Exit { code: 0 })
        })
    }
}

/// A group of one rpc leaf `run`, which nobody serves yet.
fn rpc_group(name: &str) -> GroupBuilder {
    GroupBuilder::new(name, "A group.").leaf(LeafBuilder::rpc("run", "Run.").bind(Unserved))
}

struct Unserved;

impl RpcHandler for Unserved {
    fn call(&self, _: RpcInvocation, _: RpcPort) -> LocalBoxFuture<'_, Result<u8, RpcError>> {
        unreachable!("a manifest's tree is data: the plugin host serves it")
    }
}

/// `group` as a manifest's tree at `placement`.
fn tree(placement: Placement, group: GroupBuilder) -> Commands {
    CommandPlugin::new(placement, vec![group])
        .unwrap()
        .manifest_commands()
        .remove(0)
}

fn registry(factories: Vec<Box<dyn PluginFactory>>) -> Result<Registry, RegistryError> {
    Registry::new(factories, |_| true)
}

/// Runs the rpc leaf `path` of `commands`: what it printed, or why not.
async fn run(commands: &CommandSet, path: &[&str]) -> Result<String, RpcError> {
    let memory = MemoryPort::new();
    let invocation = RpcInvocation {
        path: path.iter().map(|name| (*name).to_owned()).collect(),
        argv: Vec::new(),
        args: Default::default(),
        json: false,
        host: "laptop".into(),
        cwd: "/workspace".into(),
        env: Default::default(),
        context: test_command_context(),
        caller: None,
        stdin: false,
        pipes: None,
    };
    let port = RpcPort::new(memory.clone(), Default::default());
    commands.dispatch(invocation, port).await?;
    Ok(String::from_utf8(memory.stdout()).unwrap())
}

/// The names of the groups under the `demi` root of `commands`.
fn demi_groups(commands: &CommandSet) -> Vec<&str> {
    let demi = commands
        .declarations()
        .find(|root| root.name() == "demi")
        .expect("the set has a demi root");
    let Node::Group(demi) = demi else {
        panic!("the demi root is a group")
    };
    demi.subcommands.iter().map(Node::name).collect()
}

#[tokio::test(flavor = "local")]
async fn groups_join_the_products_under_demi_roots_stand_alone_and_each_call_gets_the_plugins_own_path()
 {
    let registry = registry(vec![
        Probe::new("notes").rpc(Placement::Demi, "notes").boxed(),
        Probe::new("lint").rpc(Placement::Root, "lint").boxed(),
    ])
    .unwrap();

    let (plugins, _data) = user_plugins(registry).await;
    let commands = plugins
        .toolset(vec![rpc_group("agent")])
        .await
        .unwrap()
        .commands;

    let roots: Vec<_> = commands.declarations().map(|root| root.name()).collect();
    assert_eq!(roots, ["demi", "lint"]);
    assert_eq!(demi_groups(&commands), ["agent", "notes"]);
    assert_eq!(
        run(&commands, &["demi", "notes", "run"]).await.unwrap(),
        "u1: notes run"
    );
    assert_eq!(
        run(&commands, &["lint", "run"]).await.unwrap(),
        "u1: lint run"
    );
}

#[test]
fn a_manifest_that_breaks_a_rule_stops_the_start_and_names_the_plugin() {
    let refusals = [
        (
            vec![Probe::new("notes").boxed(), Probe::new("notes").boxed()],
            "two plugins have the id \"notes\"",
        ),
        (
            vec![Probe::new("notes").rpc(Placement::Demi, "agent").boxed()],
            "plugin \"notes\" declares \"demi agent\", which is taken",
        ),
        (
            vec![
                Probe::new("one").rpc(Placement::Demi, "notes").boxed(),
                Probe::new("two").rpc(Placement::Demi, "notes").boxed(),
            ],
            "plugin \"two\" declares \"demi notes\", which is taken",
        ),
        (
            vec![
                Probe::new("one").rpc(Placement::Root, "lint").boxed(),
                Probe::new("two").rpc(Placement::Root, "lint").boxed(),
            ],
            "plugin \"two\"'s commands are refused",
        ),
        // The `demi` root is the plugin host's: a plugin's root of that name
        // is refused on its own, and blamed before a later group of `demi`.
        (
            vec![Probe::new("jira").rpc(Placement::Root, "demi").boxed()],
            "plugin \"jira\" declares \"demi\", which is taken",
        ),
        (
            vec![
                Probe::new("jira").rpc(Placement::Root, "demi").boxed(),
                Probe::new("notes").rpc(Placement::Demi, "notes").boxed(),
            ],
            "plugin \"jira\" declares \"demi\", which is taken",
        ),
    ];
    for (factories, refusal) in refusals {
        let error = registry(factories).err().expect("the start is refused");
        assert!(error.to_string().starts_with(refusal), "{error}");
    }
}

#[tokio::test(flavor = "local")]
async fn a_tree_bound_to_a_package_the_catalog_does_not_serve_is_left_out_whole() {
    let registry = Registry::new(
        vec![
            Probe::new("tools")
                .native("served", "demi.file")
                .native("unserved", "demi.missing")
                .boxed(),
        ],
        |operation| operation.package == "demi.file",
    )
    .unwrap();

    let (plugins, _data) = user_plugins(registry).await;
    let commands = plugins.toolset(Vec::new()).await.unwrap().commands;

    assert_eq!(demi_groups(&commands), ["served"]);
}
