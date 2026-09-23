//! The command set (`commands.md` § Declare a command): root command
//! declarations paired with the handler of each `rpc` leaf. Everything that
//! reads the tree, the manifest a runner receives and the help the model
//! reads, sees only the declarations.

use std::{collections::HashMap, rc::Rc};

use demi_command_tree::{HELP_DEFAULTS, InputSpec, Leaf, LeafKind, NativeOperation, Node};
use serde_json::Value;

use crate::{RpcError, RpcHandler, RpcInvocation, RpcPort, reserved::is_reserved};

/// A command tree's declarations, the handlers of its `rpc` leaves by path,
/// and the first thing its builders refused: what a builder hands a
/// [`CommandSet`].
pub struct Declared {
    pub(crate) tree: Node<NativeOperation>,
    /// Each handler under its leaf's path, the tree's own name first.
    pub(crate) handlers: Vec<(Vec<String>, Rc<dyn RpcHandler>)>,
    pub(crate) error: Option<String>,
}

/// A declaration the command set refuses; the text names the command and,
/// when a field is at fault, the field.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct RegisterError(String);

/// Root commands, in the order they were registered, and the handler of
/// every `rpc` leaf.
#[derive(Clone, Default)]
pub struct CommandSet {
    roots: Vec<Node<NativeOperation>>,
    handlers: HashMap<Vec<String>, Rc<dyn RpcHandler>>,
}

impl CommandSet {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a root command. It is refused, and the set unchanged, when its
    /// name is reserved or taken, when the tree breaks a declaration rule or
    /// the input subset, or when an `rpc` leaf has no handler.
    pub fn register(&mut self, root: impl Into<Declared>) -> Result<(), RegisterError> {
        let declared = root.into();
        let name = declared.tree.name().to_owned();
        if is_reserved(&name) {
            return Err(RegisterError(format!(
                "command \"{name}\" is reserved for shell and system commands"
            )));
        }
        if self.roots.iter().any(|root| root.name() == name) {
            return Err(RegisterError(format!("command \"{name}\" is already registered")));
        }
        check(&declared)?;
        self.handlers.extend(declared.handlers);
        self.roots.push(declared.tree);
        Ok(())
    }

    /// Places `node` in the group at `parent`, a path from a root, in place
    /// of the child with its name or after the others. The root is checked
    /// again as a whole and stays as it was when it is refused.
    pub fn graft(&mut self, parent: &[&str], node: impl Into<Declared>) -> Result<(), RegisterError> {
        let declared = node.into();
        let named = parent.join(" ");
        let (root_name, below) = parent
            .split_first()
            .ok_or_else(|| RegisterError("a graft names its parent group".into()))?;
        let index = self
            .roots
            .iter()
            .position(|root| root.name() == *root_name)
            .ok_or_else(|| RegisterError(format!("command \"{root_name}\" is not registered")))?;
        let mut root = self.roots[index].clone();
        let group = group_mut(&mut root, below)
            .ok_or_else(|| RegisterError(format!("\"{named}\" is not a group")))?;
        let name = declared.tree.name().to_owned();
        match group.subcommands.iter_mut().find(|child| child.name() == name) {
            Some(child) => *child = declared.tree,
            None => group.subcommands.push(declared.tree),
        }
        let prefix: Vec<String> = parent.iter().map(|name| (*name).to_owned()).collect();
        let replaced = [prefix.clone(), vec![name]].concat();
        let in_root = |path: &Vec<String>| path.first().map(String::as_str) == Some(*root_name);
        let handlers: Vec<_> = self
            .handlers
            .iter()
            .filter(|(path, _)| in_root(path) && !path.starts_with(&replaced))
            .map(|(path, handler)| (path.clone(), handler.clone()))
            .chain(
                declared
                    .handlers
                    .into_iter()
                    .map(|(path, handler)| ([prefix.clone(), path].concat(), handler)),
            )
            .collect();
        let grafted = Declared {
            tree: root,
            handlers,
            error: declared.error,
        };
        check(&grafted)?;
        self.handlers.retain(|path, _| !in_root(path));
        self.handlers.extend(grafted.handlers);
        self.roots[index] = grafted.tree;
        Ok(())
    }

    /// The commands whose leaf paths `keep` accepts; a group left without
    /// subcommands goes too.
    pub fn filter(&self, keep: impl Fn(&[String]) -> bool) -> CommandSet {
        let mut filtered = CommandSet::new();
        for root in &self.roots {
            let mut path = Vec::new();
            if let Some(tree) = keep_leaves(root, &mut path, &keep) {
                filtered.roots.push(tree);
            }
        }
        filtered.handlers = self
            .handlers
            .iter()
            .filter(|(path, _)| keep(path))
            .map(|(path, handler)| (path.clone(), handler.clone()))
            .collect();
        filtered
    }

    /// The root commands' declarations, in registration order.
    pub fn declarations(&self) -> impl Iterator<Item = &Node<NativeOperation>> {
        self.roots.iter()
    }

    /// The model's command help: the paragraph of defaults, then each root's
    /// help; nothing when the set is empty.
    pub fn render_help(&self) -> String {
        if self.roots.is_empty() {
            return String::new();
        }
        let roots: Vec<String> = self.roots.iter().map(|root| root.help(root.name())).collect();
        format!("{HELP_DEFAULTS}\n\n{}", roots.join("\n\n"))
    }

    /// Runs the `rpc` leaf `invocation.path` names. The arguments arrived as
    /// JSON and are validated as they are: no text is converted, so `"7"`
    /// for a number is a usage error.
    pub async fn dispatch(&self, invocation: RpcInvocation, port: RpcPort) -> Result<u8, RpcError> {
        let named = invocation.path.join(" ");
        let (leaf, handler) = self
            .leaf(&invocation.path)
            .zip(self.handlers.get(&invocation.path))
            .ok_or_else(|| RpcError::Usage(format!("\"{named}\" is not an rpc command")))?;
        let args = Value::Object(invocation.args.clone());
        match &leaf.input {
            Some(schema) => schema
                .check(&args)
                .map_err(|error| RpcError::Usage(format!("Invalid command arguments: {error}")))?,
            None if !invocation.args.is_empty() => {
                return Err(RpcError::Usage(format!("\"{named}\" takes no arguments")));
            }
            None => {}
        }
        handler.call(invocation, port).await
    }

    fn leaf(&self, path: &[String]) -> Option<&Leaf<NativeOperation>> {
        let (root_name, below) = path.split_first()?;
        let mut node = self.roots.iter().find(|root| root.name() == root_name)?;
        for name in below {
            let Node::Group(group) = node else {
                return None;
            };
            node = group.subcommands.iter().find(|child| child.name() == name)?;
        }
        node.leaf()
    }
}

/// Checks a declared tree: its builders' refusals, the declaration rules,
/// each leaf's input against the subset, and one handler for exactly each
/// `rpc` leaf.
fn check(declared: &Declared) -> Result<(), RegisterError> {
    let name = declared.tree.name();
    if let Some(error) = &declared.error {
        return Err(RegisterError(format!("\"{name}\": {error}")));
    }
    declared
        .tree
        .validate()
        .map_err(|error| RegisterError(format!("\"{name}\": {error}")))?;
    let mut rpc_leaves = Vec::new();
    let mut path = Vec::new();
    walk_leaves(&declared.tree, &mut path, &mut |path, leaf| {
        if let Some(schema) = &leaf.input {
            InputSpec::from_schema(schema)
                .map_err(|error| RegisterError(format!("\"{}\" {error}", path.join(" "))))?;
        }
        if leaf.kind == LeafKind::Rpc {
            rpc_leaves.push(path.to_vec());
        }
        Ok(())
    })?;
    for leaf in &rpc_leaves {
        if !declared.handlers.iter().any(|(path, _)| path == leaf) {
            return Err(RegisterError(format!(
                "rpc command \"{}\" has no handler",
                leaf.join(" ")
            )));
        }
    }
    for (path, _) in &declared.handlers {
        if !rpc_leaves.contains(path) {
            return Err(RegisterError(format!(
                "\"{}\" is not an rpc command, so it takes no handler",
                path.join(" ")
            )));
        }
    }
    Ok(())
}

/// The group at `path` below `node`.
fn group_mut<'a>(
    node: &'a mut Node<NativeOperation>,
    path: &[&str],
) -> Option<&'a mut demi_command_tree::Group<NativeOperation>> {
    let mut node = node;
    for name in path {
        node = match node {
            Node::Group(group) => group.subcommands.iter_mut().find(|child| child.name() == *name)?,
            Node::Leaf(_) => return None,
        };
    }
    match node {
        Node::Group(group) => Some(group),
        Node::Leaf(_) => None,
    }
}

fn walk_leaves(
    node: &Node<NativeOperation>,
    path: &mut Vec<String>,
    visit: &mut impl FnMut(&[String], &Leaf<NativeOperation>) -> Result<(), RegisterError>,
) -> Result<(), RegisterError> {
    path.push(node.name().to_owned());
    let result = match node {
        Node::Group(group) => group
            .subcommands
            .iter()
            .try_for_each(|child| walk_leaves(child, path, visit)),
        Node::Leaf(leaf) => visit(path, leaf),
    };
    path.pop();
    result
}

fn keep_leaves(
    node: &Node<NativeOperation>,
    path: &mut Vec<String>,
    keep: &impl Fn(&[String]) -> bool,
) -> Option<Node<NativeOperation>> {
    path.push(node.name().to_owned());
    let kept = match node {
        Node::Leaf(_) => keep(path).then(|| node.clone()),
        Node::Group(group) => {
            let subcommands: Vec<_> = group
                .subcommands
                .iter()
                .filter_map(|child| keep_leaves(child, path, keep))
                .collect();
            (!subcommands.is_empty()).then(|| {
                Node::Group(demi_command_tree::Group {
                    name: group.name.clone(),
                    summary: group.summary.clone(),
                    subcommands,
                })
            })
        }
    };
    path.pop();
    kept
}
