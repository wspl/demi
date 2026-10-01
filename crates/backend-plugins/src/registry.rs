//! The registry: every plugin the composition root registers, in that
//! order, checked once when the backend starts.

use std::{collections::BTreeSet, rc::Rc};

use demi_command_declarations::{LeafKind, NativeOperation, Node};
use demi_host_interface::{CommandSet, GroupBuilder, RegisterError};
use demi_plugin_interface::{Commands, Placement, Plugin, PluginFactory, PluginId};
use demi_shared_types::Profile;

use crate::commands::{TAKEN_GROUPS, compose};

/// Why the backend cannot start with its plugins; the error names the
/// plugin.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RegistryError {
    #[error("two plugins have the id \"{0}\"")]
    DuplicateId(PluginId),
    #[error("plugin \"{plugin}\" declares the profile \"{name}\", which {reason}")]
    Profile {
        plugin: PluginId,
        name: String,
        reason: &'static str,
    },
    #[error("plugin \"{plugin}\" declares \"{command}\", which is taken")]
    Taken { plugin: PluginId, command: String },
    #[error("plugin \"{plugin}\"'s commands are refused: {error}")]
    Commands {
        plugin: PluginId,
        error: RegisterError,
    },
}

/// One registered plugin, and the command trees it offers.
struct Registered {
    factory: Box<dyn PluginFactory>,
    /// Its trees, without those bound to a package the catalog does not
    /// serve.
    commands: Vec<Commands>,
}

/// The backend's plugins, shared by every shard thread.
pub struct Registry {
    plugins: Vec<Registered>,
    profiles: Vec<Profile>,
}

impl Registry {
    /// The registry of `factories`, in their order. A command tree with a
    /// native leaf `serves` refuses is left out whole, and logged; a
    /// manifest that breaks a rule stops the start.
    pub fn new(
        factories: Vec<Box<dyn PluginFactory>>,
        serves: impl Fn(&NativeOperation) -> bool,
    ) -> Result<Self, RegistryError> {
        let mut ids = BTreeSet::new();
        let mut profile_names = BTreeSet::new();
        let mut plugins = Vec::new();
        let mut profiles = Vec::new();
        for factory in factories {
            let manifest = factory.manifest();
            if !ids.insert(manifest.id.clone()) {
                return Err(RegistryError::DuplicateId(manifest.id.clone()));
            }
            for profile in &manifest.profiles {
                let reason = if profile.name == Profile::INHERIT {
                    Some("is reserved for inheriting the parent")
                } else if !profile_names.insert(profile.name.clone()) {
                    Some("another plugin declares")
                } else {
                    None
                };
                if let Some(reason) = reason {
                    return Err(RegistryError::Profile {
                        plugin: manifest.id.clone(),
                        name: profile.name.clone(),
                        reason,
                    });
                }
                profiles.push(profile.clone());
            }
            let commands = manifest
                .commands
                .iter()
                .filter(|commands| {
                    let served = native_operations(&commands.tree)
                        .iter()
                        .all(|op| serves(op));
                    if !served {
                        tracing::info!(
                            plugin = %manifest.id,
                            command = commands.tree.name(),
                            "a command whose package the catalog does not serve is left out"
                        );
                    }
                    served
                })
                .cloned()
                .collect();
            plugins.push(Registered { factory, commands });
        }
        let registry = Self { plugins, profiles };
        registry.check()?;
        Ok(registry)
    }

    /// Composes the command set once, with no instance behind it, so a name
    /// taken twice is found now and never while a conversation runs.
    fn check(&self) -> Result<(), RegistryError> {
        let mut demi = BTreeSet::new();
        for registered in &self.plugins {
            let plugin = &registered.factory.manifest().id;
            for commands in &registered.commands {
                let name = commands.tree.name();
                let taken = commands.placement == Placement::Demi
                    && (TAKEN_GROUPS.contains(&name) || !demi.insert(name.to_owned()));
                if taken {
                    return Err(RegistryError::Taken {
                        plugin: plugin.clone(),
                        command: format!("demi {name}"),
                    });
                }
            }
        }
        let instances: Vec<_> = self.plugins.iter().map(|_| None).collect();
        compose(self, &instances, "", Vec::new()).map(|_| ())
    }

    /// Each user's plugins: an instance of every plugin for the user's
    /// shard.
    pub fn instances(&self, user: impl Into<String>) -> UserPlugins<'_> {
        UserPlugins {
            registry: self,
            user: user.into(),
            instances: self
                .plugins
                .iter()
                .map(|registered| Some(registered.factory.instance()))
                .collect(),
        }
    }

    /// The plugins' profiles, in registration order.
    pub fn profiles(&self) -> &[Profile] {
        &self.profiles
    }

    pub(crate) fn plugins(&self) -> impl Iterator<Item = (&PluginId, &[Commands])> {
        self.plugins.iter().map(|registered| {
            (
                &registered.factory.manifest().id,
                registered.commands.as_slice(),
            )
        })
    }
}

/// One user's plugins on the user's shard.
pub struct UserPlugins<'a> {
    registry: &'a Registry,
    user: String,
    instances: Vec<Option<Rc<dyn Plugin>>>,
}

impl UserPlugins<'_> {
    /// The command set every node of the user's conversations starts from:
    /// the plugins' groups under `demi` beside the product's `product`
    /// groups, and the plugins' roots.
    pub fn commands(&self, product: Vec<GroupBuilder>) -> Result<CommandSet, RegistryError> {
        compose(self.registry, &self.instances, &self.user, product)
    }
}

/// Every native operation a tree's leaves bind.
fn native_operations(tree: &Node<NativeOperation>) -> Vec<&NativeOperation> {
    match tree {
        Node::Leaf(leaf) => match &leaf.kind {
            LeafKind::Native(operation) => vec![operation],
            LeafKind::Rpc => Vec::new(),
        },
        Node::Group(group) => group
            .subcommands
            .iter()
            .flat_map(native_operations)
            .collect(),
    }
}
