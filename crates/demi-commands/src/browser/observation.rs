//! Chrome's accessibility tree supplies names, roles, hierarchy and semantic targets.

use serde_json::json;
use std::collections::HashMap;

use chromiumoxide::{
    Element, Page,
    cdp::browser_protocol::{
        accessibility::{AxNode, AxValue, GetFullAxTreeParams, QueryAxTreeParams},
        dom::{BackendNodeId, GetDocumentParams, GetFrameOwnerParams, Node as DomNode},
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
    dom: Vec<(DomNode, FrameId)>,
    loaders: HashMap<FrameId, LoaderId>,
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
        let document = page
            .execute(GetDocumentParams::builder().depth(-1).pierce(true).build())
            .await?
            .result
            .root;
        let mut dom = Vec::new();
        let mut pending = vec![(document, main_frame)];
        while let Some((mut node, frame)) = pending.pop() {
            if let Some(document) = node.content_document.take() {
                let child_frame = node.frame_id.clone().ok_or_else(|| {
                    BrowserError::InvalidResult("frame document has no frame ID".into())
                })?;
                pending.push((*document, child_frame));
            }
            let mut children = node.children.take().unwrap_or_default();
            children.extend(node.shadow_roots.take().unwrap_or_default());
            pending.extend(
                children
                    .into_iter()
                    .rev()
                    .map(|child| (child, frame.clone())),
            );
            if node.node_type == 9 {
                // QueryAXTree computes names and roles even for hidden DOM nodes.
                let computed = page
                    .execute(
                        QueryAxTreeParams::builder()
                            .backend_node_id(node.backend_node_id)
                            .build(),
                    )
                    .await?
                    .result
                    .nodes;
                for ax in computed {
                    if let Some(existing) = nodes.iter_mut().find(|current| {
                        current.ax.backend_dom_node_id.is_some()
                            && current.ax.backend_dom_node_id == ax.backend_dom_node_id
                    }) {
                        if existing.ax.ignored {
                            existing.ax.role = ax.role;
                            existing.ax.name = ax.name;
                        }
                    } else if let Some(loader) = loaders.get(&frame) {
                        nodes.push(Node {
                            ax,
                            frame: frame.clone(),
                            loader: loader.clone(),
                            children: Vec::new(),
                        });
                    }
                }
            }
            dom.push((node, frame));
        }
        Ok(Self {
            nodes,
            order,
            dom,
            loaders,
        })
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
        let protected = node
            .ax
            .backend_dom_node_id
            .is_some_and(|backend| self.protected(backend));
        let mut states: Vec<String> = node
            .ax
            .properties
            .iter()
            .flatten()
            .filter_map(|property| match property.value.value.as_ref() {
                Some(value @ (serde_json::Value::Bool(_) | serde_json::Value::Number(_))) => {
                    Some(format!("{}={value}", property.name.as_ref()))
                }
                Some(serde_json::Value::String(value)) => {
                    Some(format!("{}={value}", property.name.as_ref()))
                }
                _ => None,
            })
            .collect();
        if protected {
            states.push("protected".into());
        }
        let value = if protected {
            None
        } else {
            node.ax
                .value
                .as_ref()
                .and_then(|value| value.value.clone())
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| BrowserError::InvalidResult(format!("AX value: {error}")))?
        };
        Ok(BrowserNode {
            r#ref: reference,
            role: ax_text(&node.ax.role).into(),
            name: ax_text(&node.ax.name).into(),
            depth,
            states,
            value,
            bounds: None,
        })
    }

    /// The native password type is reflected by its case-insensitive DOM attribute.
    /// Both inspect and value reads use this rule before returning any input value.
    pub fn protected(&self, backend: BackendNodeId) -> bool {
        self.dom.iter().any(|(node, _)| {
            node.backend_node_id == backend
                && node.local_name == "input"
                && node
                    .attributes
                    .as_deref()
                    .unwrap_or_default()
                    .as_chunks::<2>()
                    .0
                    .iter()
                    .any(|[name, value]| name == "type" && value.eq_ignore_ascii_case("password"))
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
            for (dom, _) in &self.dom {
                if dom.node_type != 1 {
                    continue;
                }
                let backend = dom.backend_node_id;
                if target.label.is_some() || target.text_match.is_some() {
                    let element = page.element_from_backend_node(backend).await?;
                    let (kind, expected) = if let Some(label) = &target.label {
                        ("label", label)
                    } else {
                        ("text", target.text_match.as_ref().expect("text locator"))
                    };
                    let matched: bool = super::element::call(
                        page,
                        &element,
                        include_str!("locator.js"),
                        vec![
                            json!(kind),
                            json!(expected),
                            json!(target.exact == Some(true)),
                        ],
                    )
                    .await?;
                    if matched {
                        matches.push(element);
                    }
                } else if let Some(node) = self
                    .nodes
                    .iter()
                    .find(|node| node.ax.backend_dom_node_id == Some(backend))
                    && target
                        .role
                        .as_ref()
                        .is_none_or(|role| ax_text(&node.ax.role) == role)
                    && target.name.as_ref().is_none_or(|name| {
                        text_matches(ax_text(&node.ax.name), name, target.exact == Some(true))
                    })
                {
                    matches.push(page.element_from_backend_node(backend).await?);
                }
            }
            matches
        };
        if let Some(container) = within {
            let container = page.element_from_backend_node(container).await?;
            let mut scoped = Vec::new();
            for element in matches {
                let response = page.evaluate_function(chromiumoxide::cdp::js_protocol::runtime::CallFunctionOnParams::builder()
                    .object_id(container.remote_object_id.clone())
                    .function_declaration("function(node) { for (; node; node = node.parentElement || node.getRootNode().host) { if (node === this) return true; } return false; }")
                    .argument(chromiumoxide::cdp::js_protocol::runtime::CallArgument::builder().object_id(element.remote_object_id.clone()).build())
                    .return_by_value(true).build().map_err(BrowserError::Configuration)?)
                    .await?.into_value::<bool>().map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
                if response {
                    scoped.push(element);
                }
            }
            matches = scoped;
        }
        if let Some(nth) = target.nth {
            matches = matches.into_iter().nth(nth as usize).into_iter().collect();
        }
        Ok(matches)
    }

    /// Removal satisfies hidden/detached only inside the reference's original document.
    pub async fn resolve_wait(
        &self,
        page: &Page,
        target: &BrowserTarget,
        refs: &References,
    ) -> Result<Vec<Element>> {
        if let Some(id) = &target.r#ref {
            let reference = refs.0.get(id).ok_or(BrowserError::StaleReference)?;
            if self.loaders.get(&reference.frame) != Some(&reference.loader) {
                return Err(BrowserError::StaleReference);
            }
            if !self.dom.iter().any(|(node, frame)| {
                node.backend_node_id == reference.backend && *frame == reference.frame
            }) {
                return Ok(Vec::new());
            }
        }
        self.resolve(page, target, refs).await
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
                    .position(|node| node.ax.backend_dom_node_id == Some(element.backend_node_id));
                if let Some(index) = index {
                    self.describe(index, 0, refs)
                } else {
                    let (_, frame) = self
                        .dom
                        .iter()
                        .find(|(node, _)| node.backend_node_id == element.backend_node_id)
                        .ok_or(BrowserError::StaleReference)?;
                    let loader = self
                        .loaders
                        .get(frame)
                        .ok_or(BrowserError::StaleReference)?;
                    Ok(BrowserNode {
                        r#ref: Some(refs.issue(Reference {
                            backend: element.backend_node_id,
                            frame: frame.clone(),
                            loader: loader.clone(),
                        })?),
                        role: String::new(),
                        name: String::new(),
                        depth: 0,
                        states: Vec::new(),
                        bounds: None,
                        value: None,
                    })
                }
            })
            .collect()
    }

    fn reference(&self, id: &str, refs: &References) -> Result<BackendNodeId> {
        let reference = refs.0.get(id).ok_or(BrowserError::StaleReference)?;
        if self.loaders.get(&reference.frame) != Some(&reference.loader) {
            return Err(BrowserError::StaleReference);
        }
        self.dom
            .iter()
            .find(|(node, frame)| {
                node.backend_node_id == reference.backend && *frame == reference.frame
            })
            .map(|_| reference.backend)
            .ok_or(BrowserError::StaleReference)
    }
}

impl References {
    pub fn invalidate(&mut self) {
        self.0.clear();
    }

    fn issue(&mut self, reference: Reference) -> Result<String> {
        if let Some((id, _)) = self.0.iter().find(|(_, current)| **current == reference) {
            return Ok(id.clone());
        }
        if self.0.len() >= 10_000 {
            return Err(BrowserError::Configuration(
                "browser reference limit reached for this document".into(),
            ));
        }
        let id = super::handles::fresh("e")?;
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
    let count = locator_count(target);
    if count != 1 || (target.name.is_some() && target.role.is_none()) {
        return Err(BrowserError::Configuration(
            "provide exactly one of ref, role/name, label, placeholder, text-match, test-id or css"
                .into(),
        ));
    }
    Ok(())
}

/// Count only the primary locators in the browser's shared target grammar.
pub(super) fn locator_count(target: &BrowserTarget) -> usize {
    [
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
    .count()
}

/// URL and coordinate targets cannot silently ignore supplied element flags.
pub(super) fn has_target_flags(target: &BrowserTarget) -> bool {
    locator_count(target) > 0
        || target.name.is_some()
        || target.exact.is_some()
        || target.nth.is_some()
        || target.within.is_some()
}
