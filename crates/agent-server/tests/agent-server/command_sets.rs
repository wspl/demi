//! The commands a node holds (`runtime.md` § What a session holds, §
//! Acceptance): every node of the user with the same plugins and the same
//! right to spawn holds one set, composed once per revision of the plugins
//! and kept while a node holds it.

use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

use demi_agent_server::{AgentServer, ServerConfig, ServerDeps, testing::ScriptedProviders};
use demi_agent_store::{AgentTreeStore, testing::MemoryTreeStore};
use demi_agent_tools::{Profile, SubagentSettings, Toolset, ToolsetSource};
use demi_agent_transcript::testing::SequentialIds;
use demi_host_interface::CommandSet;
use demi_provider_common::testing::FixedClock;
use demi_shared_types::NodeId;
use futures_util::future::LocalBoxFuture;
use serde_json::json;

use crate::subagents::held_said;
use crate::support::{Gate, LiveHosts, Model, ScriptedShells, agent_in, named_node, start};

/// The user's plugins: a revision the test changes, as turning a plugin on
/// does, and how many times the commands were composed.
struct Plugins {
    revision: RefCell<Rc<str>>,
    composed: Cell<usize>,
}

impl ToolsetSource for Plugins {
    fn revision(&self) -> LocalBoxFuture<'_, Result<Rc<str>, String>> {
        let revision = self.revision.borrow().clone();
        Box::pin(async move { Ok(revision) })
    }

    fn current(&self) -> LocalBoxFuture<'_, Result<Toolset, String>> {
        self.composed.set(self.composed.get() + 1);
        let revision = self.revision.borrow().clone();
        Box::pin(async move {
            Ok(Toolset {
                commands: Rc::new(CommandSet::new()),
                revision,
            })
        })
    }
}

/// The commands the node `node` of the live tree `root` holds.
fn commands_of(server: &Rc<AgentServer<LiveHosts>>, root: &NodeId, node: &NodeId) -> Rc<CommandSet> {
    server
        .node(root, node)
        .unwrap_or_else(|| panic!("no live node {node}"))
        .commands()
        .clone()
}

/// Cost: four trees of one root and at most one child each, in memory.
#[tokio::test(flavor = "local")]
async fn trees_with_the_same_plugins_share_one_command_set_per_spawning_right() {
    let gate = Gate::new();
    let model = Model::default();
    model.child("task a", [held_said(&gate, "a done")]);
    model.child("task b", [held_said(&gate, "b done")]);
    let providers = Rc::new(ScriptedProviders::default());
    providers.provide_runtime("stub", model.clone());
    let plugins = Rc::new(Plugins {
        revision: RefCell::new(Rc::from("web")),
        composed: Cell::new(0),
    });
    let store = MemoryTreeStore::new();
    let server = {
        let store = store.clone();
        AgentServer::new(ServerDeps {
            toolsets: plugins.clone(),
            // The children of the profile `leaf` may not spawn: they hold
            // the other right.
            subagents: Rc::new(SubagentSettings {
                enabled: true,
                max_live: None,
                max_depth: None,
                profiles: vec![Profile {
                    name: "leaf".into(),
                    description: "Does one thing.".into(),
                    model: None,
                    context_limit: None,
                    instructions: None,
                    can_spawn: false,
                    enabled: true,
                }],
            }),
            instructions: Rc::from("system prompt"),
            guide: Rc::from("harness guide"),
            hosts: Rc::new(LiveHosts::default()),
            context: Rc::new([]),
            providers,
            shells: Rc::new(ScriptedShells::default()),
            stores: Rc::new(move |_: &NodeId| store.clone() as Rc<dyn AgentTreeStore>),
            clock: Arc::new(FixedClock(start())),
            ids: Rc::new(SequentialIds::new("id")),
            config: ServerConfig::default(),
            status_changed: Rc::new(|_| {}),
        })
    };
    let first = NodeId::try_from("first").unwrap();
    let second = NodeId::try_from("second").unwrap();
    let third = NodeId::try_from("third").unwrap();

    // Two trees with the same plugins, each with a child.
    server.restore(&first).await.expect("the first tree opens");
    server.restore(&second).await.expect("the second tree opens");
    let spawned = agent_in(&server, &first, &first, "spawn", json!({ "prompt": "task a", "profile": "leaf" })).await;
    assert_eq!(spawned.code, 0, "{spawned:?}");
    let first_child = named_node(&store, &spawned);
    let spawned = agent_in(&server, &second, &second, "spawn", json!({ "prompt": "task b", "profile": "leaf" })).await;
    assert_eq!(spawned.code, 0, "{spawned:?}");
    let second_child = named_node(&store, &spawned);

    let first_root = commands_of(&server, &first, &first);
    let first_children = commands_of(&server, &first, &first_child);
    assert!(Rc::ptr_eq(&first_root, &commands_of(&server, &second, &second)));
    assert!(Rc::ptr_eq(&first_children, &commands_of(&server, &second, &second_child)));
    assert!(!Rc::ptr_eq(&first_root, &first_children), "a child that may not spawn holds the other set");
    assert_eq!(plugins.composed.get(), 1);

    // A plugin turned on: the next tree composes the new revision,
    // and the open trees keep what they hold.
    plugins.revision.replace(Rc::from("web,skills"));
    server.restore(&third).await.expect("the third tree opens");
    assert!(!Rc::ptr_eq(&first_root, &commands_of(&server, &third, &third)));
    assert_eq!(plugins.composed.get(), 2);
    assert!(Rc::ptr_eq(&first_root, &commands_of(&server, &first, &first)));
    assert!(Rc::ptr_eq(&first_root, &commands_of(&server, &second, &second)));
    assert!(Rc::ptr_eq(&first_children, &commands_of(&server, &second, &second_child)));

    // The plugin turned off again: a tree that opens now shares the set
    // the first two still hold, composed for that revision before.
    plugins.revision.replace(Rc::from("web"));
    let fourth = NodeId::try_from("fourth").unwrap();
    server.restore(&fourth).await.expect("the fourth tree opens");
    assert!(Rc::ptr_eq(&first_root, &commands_of(&server, &fourth, &fourth)));
    assert_eq!(plugins.composed.get(), 2);

    // Once the last tree that holds a set closes, it is freed: the next
    // open composes again.
    drop((first_root, first_children));
    for root in [&first, &second, &third, &fourth] {
        server.close_tree(root).await;
    }
    server.restore(&first).await.expect("the first tree opens again");
    assert_eq!(plugins.composed.get(), 3);
    gate.open();
}
