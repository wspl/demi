//! The registry: every plugin the composition root registers, in that
//! order, checked once when the backend starts. What the startup catalog
//! does not serve is left out: a command tree, a user stream or a page
//! method bound to an operation the catalog lacks.

use std::collections::BTreeSet;

use demi_command_declarations::{LeafKind, NativeOperation, Node};
use demi_host_interface::RegisterError;
use demi_plugin_interface::{
    Commands, Follows, Manifest, Page, Placement, PluginFactory, PluginId, Stream,
};
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
    #[error("plugin \"{plugin}\" declares the user stream \"{name}\", which is taken")]
    Stream { plugin: PluginId, name: String },
    #[error("plugin \"{plugin}\"'s commands are refused: {error}")]
    Commands {
        plugin: PluginId,
        error: RegisterError,
    },
}

/// One registered plugin, and what of its manifest the catalog serves.
pub(crate) struct Registered {
    pub(crate) factory: Box<dyn PluginFactory>,
    pub(crate) commands: Vec<Commands>,
    pub(crate) streams: Vec<Stream>,
    pub(crate) page: Option<Page>,
    /// The packages its served commands, streams and methods bind: the
    /// only ones its package calls may name.
    pub(crate) packages: BTreeSet<String>,
}

impl Registered {
    pub(crate) fn id(&self) -> &PluginId {
        &self.factory.manifest().id
    }
}

/// The backend's plugins, shared by every shard thread.
pub struct Registry {
    pub(crate) plugins: Vec<Registered>,
    profiles: Vec<Profile>,
}

impl Registry {
    /// The registry of `factories`, in their order. What binds an operation
    /// `serves` refuses is left out, and logged; a manifest that breaks a
    /// rule stops the start.
    pub fn new(
        factories: Vec<Box<dyn PluginFactory>>,
        serves: impl Fn(&NativeOperation) -> bool,
    ) -> Result<Self, RegistryError> {
        let mut ids = BTreeSet::new();
        let mut profile_names = BTreeSet::new();
        let mut stream_names = BTreeSet::new();
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
            for stream in &manifest.streams {
                if !stream_names.insert(stream.name.clone()) {
                    return Err(RegistryError::Stream {
                        plugin: manifest.id.clone(),
                        name: stream.name.clone(),
                    });
                }
            }
            plugins.push(served(factory, &serves));
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
            for commands in &registered.commands {
                let name = commands.tree.name();
                let taken = commands.placement == Placement::Demi
                    && (TAKEN_GROUPS.contains(&name) || !demi.insert(name.to_owned()));
                if taken {
                    return Err(RegistryError::Taken {
                        plugin: registered.id().clone(),
                        command: format!("demi {name}"),
                    });
                }
            }
        }
        compose(self, None, Vec::new(), |_| true).map(|_| ())
    }

    /// The plugins' profiles, in registration order.
    pub fn profiles(&self) -> &[Profile] {
        &self.profiles
    }

    /// The plugin that declares the user stream `name`, by its index.
    pub(crate) fn stream_owner(&self, name: &str) -> Option<usize> {
        self.plugins
            .iter()
            .position(|registered| registered.streams.iter().any(|stream| stream.name == name))
    }

    /// Every user stream the catalog serves, of every plugin.
    pub fn streams(&self) -> impl Iterator<Item = &Stream> {
        self.plugins
            .iter()
            .flat_map(|registered| &registered.streams)
    }

    /// The plugins whose page state follows `change`, in registration
    /// order.
    pub fn followers(&self, change: Follows) -> impl Iterator<Item = &PluginId> {
        self.plugins
            .iter()
            .filter(move |registered| {
                registered
                    .page
                    .as_ref()
                    .is_some_and(|page| page.follows.contains(&change))
            })
            .map(Registered::id)
    }

    pub(crate) fn plugin(&self, id: &str) -> Option<(usize, &Registered)> {
        self.plugins
            .iter()
            .enumerate()
            .find(|(_, registered)| registered.id().as_str() == id)
    }
}

/// `factory` with what of its manifest `serves` serves.
fn served(
    factory: Box<dyn PluginFactory>,
    serves: &impl Fn(&NativeOperation) -> bool,
) -> Registered {
    let manifest: &Manifest = factory.manifest();
    let left_out = |what: &str, name: &str| {
        tracing::info!(
            plugin = %manifest.id,
            what,
            name,
            "a part whose package the catalog does not serve is left out"
        );
    };
    let mut packages = BTreeSet::new();
    let mut bound = |operations: &[&NativeOperation]| {
        packages.extend(operations.iter().map(|operation| operation.package.clone()));
    };
    let mut commands = Vec::new();
    for tree in &manifest.commands {
        let operations = native_operations(&tree.tree);
        if operations.iter().all(|operation| serves(operation)) {
            bound(&operations);
            commands.push(tree.clone());
        } else {
            left_out("command", tree.tree.name());
        }
    }
    let mut streams = Vec::new();
    for stream in &manifest.streams {
        if serves(&stream.operation) {
            bound(&[&stream.operation]);
            streams.push(stream.clone());
        } else {
            left_out("user stream", &stream.name);
        }
    }
    let page = manifest.page.as_ref().map(|page| {
        let mut page = page.clone();
        page.methods.retain(|method| {
            let operations: Vec<&NativeOperation> = method.operations.iter().collect();
            let kept = operations.iter().all(|operation| serves(operation));
            if kept {
                bound(&operations);
            } else {
                left_out("page method", &method.name);
            }
            kept
        });
        page
    });
    Registered {
        commands,
        streams,
        page,
        packages,
        factory,
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
