//! Chrome's accessibility tree supplies names, roles, hierarchy and semantic targets.

use std::collections::HashMap;

use chromiumoxide::{
    Element, Page,
    cdp::browser_protocol::{
        accessibility::{AxNode, AxValue, GetFullAxTreeParams},
        dom::{BackendNodeId, GetFrameOwnerParams},
        network::LoaderId,
        page::{FrameId, GetFrameTreeParams},
    },
};

use super::{
    BrowserError, Result,
    protocol::{BrowserNode, BrowserTarget},
};

#[derive(Clone, PartialEq)]
struct Reference {
    backend: BackendNodeId,
    frame: FrameId,
    loader: LoaderId,
}

#[derive(Default)]
pub(super) struct References(HashMap<String, Reference>);

struct Node {
    ax: AxNode,
    frame: FrameId,
    loader: LoaderId,
    children: Vec<usize>,
}

pub(super) struct Observation {
    nodes: Vec<Node>,
    order: Vec<(usize, u64)>,
}

impl Observation {
    /// Join each frame's AX tree at its iframe element, preserving document identity.
    pub async fn capture(page: &Page, refs: &mut References) -> Result<Self> {
        let tree = page.execute(GetFrameTreeParams {}).await?.result.frame_tree;
        let main_frame = tree.frame.id.clone();
        let mut frames = vec![tree];
        let mut nodes = Vec::new();
        let mut roots = HashMap::new();
        let mut owners = Vec::new();
        let mut loaders = HashMap::new();
        while let Some(tree) = frames.pop() {
            let frame = tree.frame;
            if frame.parent_id.is_some() {
                let owner = page
                    .execute(GetFrameOwnerParams::new(frame.id.clone()))
                    .await?
                    .result
                    .backend_node_id;
                owners.push((frame.id.clone(), owner));
            }
            loaders.insert(frame.id.clone(), frame.loader_id.clone());
            let ax = page
                .execute(
                    GetFullAxTreeParams::builder()
                        .frame_id(frame.id.clone())
                        .build(),
                )
                .await?
                .result
                .nodes;
            let first = nodes.len();
            let indices: HashMap<_, _> = ax
                .iter()
                .enumerate()
                .map(|(index, node)| (node.node_id.clone(), first + index))
                .collect();
            if !ax.is_empty() {
                roots.insert(frame.id.clone(), first);
            }
            for node in ax {
                let children = node
                    .child_ids
                    .iter()
                    .flatten()
                    .filter_map(|id| indices.get(id).copied())
                    .collect();
                nodes.push(Node {
                    ax: node,
                    frame: frame.id.clone(),
                    loader: frame.loader_id.clone(),
                    children,
                });
            }
            frames.extend(tree.child_frames.unwrap_or_default());
        }
        refs.0
            .retain(|_, reference| loaders.get(&reference.frame) == Some(&reference.loader));
        for (frame, backend) in owners {
            if let (Some(root), Some(owner)) = (
                roots.get(&frame),
                nodes
                    .iter_mut()
                    .find(|node| node.ax.backend_dom_node_id == Some(backend)),
            ) {
                owner.children.push(*root);
            }
        }
        let mut order = Vec::new();
        let mut pending: Vec<_> = roots
            .get(&main_frame)
            .map(|root| (*root, 0))
            .into_iter()
            .collect();
        while let Some((index, depth)) = pending.pop() {
            let node = &nodes[index];
            let visible = !node.ax.ignored && ax_text(&node.ax.role) != "InlineTextBox";
            if visible {
                order.push((index, depth));
            }
            for child in node.children.iter().rev() {
                pending.push((*child, depth + u64::from(visible)));
            }
        }
        Ok(Self { nodes, order })
    }

    pub fn tree(&self, refs: &mut References, limit: usize) -> Result<(Vec<BrowserNode>, bool)> {
        let nodes = self
            .order
            .iter()
            .take(limit)
            .map(|(index, depth)| self.describe(*index, *depth, refs))
            .collect::<Result<_>>()?;
        Ok((nodes, self.order.len() > limit))
    }

    fn describe(&self, index: usize, depth: u64, refs: &mut References) -> Result<BrowserNode> {
        let node = &self.nodes[index];
        let reference = match node.ax.backend_dom_node_id {
            Some(backend) => Some(refs.issue(Reference {
                backend,
                frame: node.frame.clone(),
                loader: node.loader.clone(),
            })?),
            None => None,
        };
        let states = node
            .ax
            .properties
            .iter()
            .flatten()
            .filter_map(|property| match property.value.value.as_ref() {
                Some(serde_json::Value::Bool(true)) => Some(property.name.as_ref().to_owned()),
                Some(serde_json::Value::String(value)) if !value.is_empty() && value != "false" => {
                    Some(format!("{}={value}", property.name.as_ref()))
                }
                _ => None,
            })
            .collect();
        Ok(BrowserNode {
            r#ref: reference,
            role: ax_text(&node.ax.role).into(),
            name: ax_text(&node.ax.name).into(),
            depth,
            states,
            bounds: None,
        })
    }

    /// Resolve fresh semantic/CSS matches against the same observation used for output.
    pub async fn resolve(
        &self,
        page: &Page,
        target: &BrowserTarget,
        refs: &References,
    ) -> Result<Vec<Element>> {
        validate_target(target)?;
        let within = match &target.within {
            Some(reference) => Some(self.reference(reference, refs)?),
            None => None,
        };
        let mut matches = if let Some(reference) = &target.r#ref {
            vec![
                page.element_from_backend_node(self.reference(reference, refs)?)
                    .await?,
            ]
        } else if target.css.is_some() || target.placeholder.is_some() || target.test_id.is_some() {
            let selector = match (&target.css, &target.placeholder, &target.test_id) {
                (Some(css), _, _) => css.clone(),
                (_, Some(value), _) => format!(
                    "[placeholder={}]",
                    serde_json::to_string(value)
                        .map_err(|error| BrowserError::Configuration(error.to_string()))?
                ),
                (_, _, Some(value)) => format!(
                    "[data-testid={}]",
                    serde_json::to_string(value)
                        .map_err(|error| BrowserError::Configuration(error.to_string()))?
                ),
                _ => unreachable!("selector branch checks its alternatives"),
            };
            match within {
                Some(backend) => {
                    page.element_from_backend_node(backend)
                        .await?
                        .find_elements(selector)
                        .await?
                }
                None => page.find_elements(selector).await?,
            }
        } else {
            let mut matches = Vec::new();
            for (index, _) in &self.order {
                let node = &self.nodes[*index];
                let Some(backend) = node.ax.backend_dom_node_id else {
                    continue;
                };
                let role = ax_text(&node.ax.role);
                let name = ax_text(&node.ax.name);
                let eligible = target.role.as_ref().is_none_or(|expected| role == expected)
                    && target.name.as_ref().is_none_or(|expected| {
                        text_matches(name, expected, target.exact == Some(true))
                    })
                    && target.label.as_ref().is_none_or(|expected| {
                        matches!(
                            role,
                            "textbox"
                                | "combobox"
                                | "checkbox"
                                | "radio"
                                | "searchbox"
                                | "spinbutton"
                                | "slider"
                                | "switch"
                        ) && text_matches(name, expected, target.exact == Some(true))
                    })
                    && target.text_match.as_ref().is_none_or(|expected| {
                        text_matches(name, expected, target.exact == Some(true))
                    });
                if eligible {
                    matches.push(page.element_from_backend_node(backend).await?);
                }
            }
            matches
        };
        if let Some(container) = within {
            let mut descendants = Vec::new();
            let mut pending: Vec<_> = self
                .nodes
                .iter()
                .position(|node| node.ax.backend_dom_node_id == Some(container))
                .into_iter()
                .collect();
            while let Some(index) = pending.pop() {
                descendants.extend(self.nodes[index].ax.backend_dom_node_id);
                pending.extend(&self.nodes[index].children);
            }
            matches.retain(|element| descendants.contains(&element.backend_node_id));
        }
        if let Some(nth) = target.nth {
            matches = matches.into_iter().nth(nth as usize).into_iter().collect();
        }
        Ok(matches)
    }

    pub fn describe_elements(
        &self,
        elements: &[Element],
        refs: &mut References,
        limit: usize,
    ) -> Result<Vec<BrowserNode>> {
        elements
            .iter()
            .take(limit)
            .map(|element| {
                let index = self
                    .nodes
                    .iter()
                    .position(|node| node.ax.backend_dom_node_id == Some(element.backend_node_id))
                    .ok_or_else(|| {
                        BrowserError::InvalidResult(
                            "matched element is absent from the accessibility tree".into(),
                        )
                    })?;
                self.describe(index, 0, refs)
            })
            .collect()
    }

    fn reference(&self, id: &str, refs: &References) -> Result<BackendNodeId> {
        let reference = refs.0.get(id).ok_or(BrowserError::StaleReference)?;
        self.nodes
            .iter()
            .find(|node| {
                node.ax.backend_dom_node_id == Some(reference.backend)
                    && node.frame == reference.frame
                    && node.loader == reference.loader
            })
            .map(|_| reference.backend)
            .ok_or(BrowserError::StaleReference)
    }
}

impl References {
    fn issue(&mut self, reference: Reference) -> Result<String> {
        if let Some((id, _)) = self.0.iter().find(|(_, current)| **current == reference) {
            return Ok(id.clone());
        }
        if self.0.len() >= 10_000 {
            return Err(BrowserError::Configuration(
                "browser reference limit reached for this document".into(),
            ));
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        self.0.insert(id.clone(), reference);
        Ok(id)
    }
}

/// Chrome AX values carry optional JSON; absent name/role means empty text.
fn ax_text(value: &Option<AxValue>) -> &str {
    value
        .as_ref()
        .and_then(|value| value.value.as_ref())
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
}

/// Match the browser command's literal substring or exact semantic text contract.
fn text_matches(actual: &str, expected: &str, exact: bool) -> bool {
    if exact {
        actual == expected
    } else {
        actual.contains(expected)
    }
}

/// Locator alternatives cannot silently override each other or use modifier-only targets.
pub(super) fn validate_target(target: &BrowserTarget) -> Result<()> {
    let count = [
        target.r#ref.is_some(),
        target.role.is_some(),
        target.label.is_some(),
        target.placeholder.is_some(),
        target.text_match.is_some(),
        target.test_id.is_some(),
        target.css.is_some(),
    ]
    .into_iter()
    .filter(|value| *value)
    .count();
    if count != 1 || (target.name.is_some() && target.role.is_none()) {
        return Err(BrowserError::Configuration(
            "provide exactly one of ref, role/name, label, placeholder, text-match, test-id or css"
                .into(),
        ));
    }
    Ok(())
}
