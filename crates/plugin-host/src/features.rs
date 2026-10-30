//! A plugin's features: the parts of its command groups each user turns on
//! and off apart ([`Feature`]).
//!
//! The registry checks a manifest's features once, against the trees the
//! manifest declares: each has an id of lowercase letters, digits and
//! hyphens, names a group that one tree declares and one or more leaves of
//! that group, no leaf is two features', and a group's features hold all its
//! subcommands, which are leaves. The composer then narrows a tree by the features a user has
//! off: their leaves go, and the group says the sentences of the features
//! left on; a group with none on goes, and a group it leaves empty with it.

use std::collections::BTreeSet;

use demi_command_declarations::{Group, NativeOperation, Node};
use demi_plugin_interface::{Commands, Feature};

/// Why a manifest's features are refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum FeatureError {
    #[error("two features are named \"{0}\"")]
    Duplicate(String),
    #[error("the feature id \"{0}\" is not lowercase letters, digits and hyphens")]
    Id(String),
    #[error("feature \"{0}\" holds no leaf")]
    Empty(String),
    #[error("feature \"{feature}\" names the group {group:?}, which the plugin's trees do not declare")]
    Group { feature: String, group: Vec<String> },
    #[error("feature \"{feature}\" names the group {group:?}, which two of the plugin's trees declare")]
    Ambiguous { feature: String, group: Vec<String> },
    #[error("feature \"{feature}\" names \"{leaf}\", which is no leaf of its group")]
    Leaf { feature: String, leaf: String },
    #[error("the leaf \"{leaf}\" is two features'")]
    Shared { leaf: String },
    #[error("the group {group:?} has \"{name}\", which is no leaf one of its features holds")]
    Uncovered { group: Vec<String>, name: String },
}

/// Checks `features` against the trees of `commands`.
pub(crate) fn check(commands: &[Commands], features: &[Feature]) -> Result<(), FeatureError> {
    let mut ids = BTreeSet::new();
    let mut held = BTreeSet::new();
    for feature in features {
        let id_ok = !feature.id.is_empty()
            && feature
                .id
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
        if !id_ok {
            return Err(FeatureError::Id(feature.id.clone()));
        }
        if !ids.insert(feature.id.as_str()) {
            return Err(FeatureError::Duplicate(feature.id.clone()));
        }
        if feature.leaves.is_empty() {
            return Err(FeatureError::Empty(feature.id.clone()));
        }
        let mut groups = commands
            .iter()
            .filter_map(|commands| group_at(&commands.tree, &feature.group));
        let group = groups.next().ok_or_else(|| FeatureError::Group {
            feature: feature.id.clone(),
            group: feature.group.clone(),
        })?;
        if groups.next().is_some() {
            return Err(FeatureError::Ambiguous {
                feature: feature.id.clone(),
                group: feature.group.clone(),
            });
        }
        for leaf in &feature.leaves {
            let is_leaf = group
                .subcommands
                .iter()
                .any(|child| matches!(child, Node::Leaf(declared) if &declared.name == leaf));
            if !is_leaf {
                return Err(FeatureError::Leaf {
                    feature: feature.id.clone(),
                    leaf: leaf.clone(),
                });
            }
            if !held.insert((feature.group.clone(), leaf.clone())) {
                return Err(FeatureError::Shared { leaf: leaf.clone() });
            }
        }
    }
    for feature in features {
        let group = commands
            .iter()
            .find_map(|commands| group_at(&commands.tree, &feature.group))
            .expect("checked above");
        for child in &group.subcommands {
            if !held.contains(&(feature.group.clone(), child.name().to_owned())) {
                return Err(FeatureError::Uncovered {
                    group: feature.group.clone(),
                    name: child.name().to_owned(),
                });
            }
        }
    }
    Ok(())
}

/// The group at `path` from `tree`, whose own name is the path's first.
fn group_at<'a>(tree: &'a Node<NativeOperation>, path: &[String]) -> Option<&'a Group<NativeOperation>> {
    let (first, rest) = path.split_first()?;
    let Node::Group(group) = tree else {
        return None;
    };
    if &group.name != first {
        return None;
    }
    match rest.first() {
        None => Some(group),
        Some(_) => group.subcommands.iter().find_map(|child| group_at(child, rest)),
    }
}

/// `tree` without the leaves of the features `on` turns down, its groups
/// saying the sentences of those left on; none when nothing is left.
pub(crate) fn narrow(
    tree: &Node<NativeOperation>,
    features: &[Feature],
    on: impl Fn(&Feature) -> bool,
) -> Option<Node<NativeOperation>> {
    if features.iter().all(&on) {
        return Some(tree.clone());
    }
    narrow_at(tree, &mut Vec::new(), features, &on)
}

fn narrow_at(
    node: &Node<NativeOperation>,
    path: &mut Vec<String>,
    features: &[Feature],
    on: &impl Fn(&Feature) -> bool,
) -> Option<Node<NativeOperation>> {
    let Node::Group(group) = node else {
        return Some(node.clone());
    };
    path.push(group.name.clone());
    let own: Vec<&Feature> = features.iter().filter(|feature| feature.group == *path).collect();
    let narrowed = if own.is_empty() {
        let subcommands: Vec<_> = group
            .subcommands
            .iter()
            .filter_map(|child| narrow_at(child, path, features, on))
            .collect();
        (!subcommands.is_empty()).then(|| {
            Node::Group(Group {
                subcommands,
                ..group.clone()
            })
        })
    } else if own.iter().all(|feature| on(feature)) {
        Some(node.clone())
    } else {
        let kept: Vec<&Feature> = own.into_iter().filter(|feature| on(feature)).collect();
        let leaves: BTreeSet<&str> = kept
            .iter()
            .flat_map(|feature| feature.leaves.iter().map(String::as_str))
            .collect();
        let joined = |sentence: fn(&Feature) -> &str| {
            kept.iter().map(|feature| sentence(feature)).collect::<Vec<_>>().join(" ")
        };
        (!kept.is_empty()).then(|| {
            Node::Group(Group {
                summary: joined(|feature| &feature.summary),
                index_entry: group
                    .index_entry
                    .as_ref()
                    .map(|_| joined(|feature| &feature.index_entry)),
                subcommands: group
                    .subcommands
                    .iter()
                    .filter(|child| leaves.contains(child.name()))
                    .cloned()
                    .collect(),
                ..group.clone()
            })
        })
    };
    path.pop();
    narrowed
}
