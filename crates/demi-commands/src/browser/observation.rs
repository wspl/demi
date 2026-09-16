//! Chrome's accessibility tree supplies names, roles, hierarchy and semantic targets.

use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;

use chromiumoxide::{
    Page,
    cdp::browser_protocol::{
        accessibility::{AxNode, AxValue, GetFullAxTreeParams},
        dom::{BackendNodeId, GetDocumentParams, GetFrameOwnerParams, Node as DomNode},
        network::LoaderId,
        page::{FrameId, GetFrameTreeParams},
    },
};

use super::{
    BrowserError, Result,
    element::TargetElement,
    protocol::{BrowserNode, BrowserTarget},
};

#[derive(Clone, PartialEq, Eq, Hash)]
struct Reference {
    backend: BackendNodeId,
    frame: FrameId,
    loader: LoaderId,
}

#[derive(Default)]
pub(super) struct References {
    by_id: HashMap<String, Reference>,
    by_node: HashMap<Reference, String>,
}

struct Node {
    ax: AxNode,
    frame: FrameId,
    loader: LoaderId,
    children: Vec<usize>,
}

#[derive(Default)]
pub(super) struct Observation {
    nodes: Vec<Node>,
    order: Vec<(usize, u64)>,
    dom: Vec<(DomNode, FrameId)>,
    loaders: HashMap<FrameId, LoaderId>,
    dom_index: HashMap<BackendNodeId, usize>,
    ax_index: HashMap<BackendNodeId, usize>,
    parents: HashMap<BackendNodeId, BackendNodeId>,
    documents: Vec<FrameId>,
}

impl Observation {
    /// Join each frame's AX tree at its iframe element, preserving document identity.
    pub async fn capture(page: &Page, refs: &mut References) -> Result<Self> {
        let tree = page.execute(GetFrameTreeParams {}).await?.result.frame_tree;
        let main_frame = tree.frame.id.clone();
        let mut frames = vec![tree];
        let mut nodes = Vec::new();
        let mut ax_index = HashMap::new();
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
                if let Some(backend) = node.backend_dom_node_id {
                    ax_index.entry(backend).or_insert(nodes.len());
                }
                nodes.push(Node {
                    ax: node,
                    frame: frame.id.clone(),
                    loader: frame.loader_id.clone(),
                    children,
                });
            }
            frames.extend(tree.child_frames.unwrap_or_default());
        }
        refs.by_id
            .retain(|_, reference| loaders.get(&reference.frame) == Some(&reference.loader));
        refs.by_node.retain(|_, id| refs.by_id.contains_key(id));
        for (frame, backend) in owners {
            if let (Some(root), Some(owner)) = (
                roots.get(&frame),
                ax_index.get(&backend).map(|index| &mut nodes[*index]),
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
        let mut dom_index = HashMap::new();
        let mut parents = HashMap::new();
        let mut documents = Vec::new();
        let mut pending = vec![(document, main_frame, None)];
        while let Some((mut node, frame, parent)) = pending.pop() {
            let backend = node.backend_node_id;
            if let Some(parent) = parent {
                parents.insert(backend, parent);
            }
            if let Some(document) = node.content_document.take() {
                let child_frame = node.frame_id.clone().ok_or_else(|| {
                    BrowserError::InvalidResult("frame document has no frame ID".into())
                })?;
                pending.push((*document, child_frame, Some(backend)));
            }
            let mut children = node.children.take().unwrap_or_default();
            children.extend(node.shadow_roots.take().unwrap_or_default());
            pending.extend(
                children
                    .into_iter()
                    .rev()
                    .map(|child| (child, frame.clone(), Some(backend))),
            );
            if node.node_type == 9 {
                documents.push(frame.clone());
            }
            dom_index.insert(backend, dom.len());
            dom.push((node, frame));
        }
        Ok(Self {
            nodes,
            order,
            dom,
            loaders,
            dom_index,
            ax_index,
            parents,
            documents,
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
                .and_then(|value| value.value.as_ref())
                .filter(|value| value.is_string() || value.is_number())
                .cloned()
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
        self.dom_index.get(&backend).is_some_and(|index| {
            let node = &self.dom[*index].0;
            node.local_name == "input"
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
    ) -> Result<Vec<TargetElement>> {
        validate_target(target)?;
        let within = match &target.within {
            Some(reference) => Some(self.reference(reference, refs)?),
            None => None,
        };
        let mut matches = if let Some(reference) = &target.r#ref {
            vec![TargetElement::resolve(page, self.reference(reference, refs)?).await?]
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
            let elements = match within {
                Some(backend) => {
                    page.element_from_backend_node(backend)
                        .await?
                        .find_elements(selector)
                        .await?
                }
                None => page.find_elements(selector).await?,
            };
            elements.into_iter().map(TargetElement::from).collect()
        } else if target.label.is_some() || target.text_match.is_some() {
            self.text_targets(page, target).await?
        } else {
            let mut matches = Vec::new();
            for (dom, _) in &self.dom {
                if dom.node_type != 1 {
                    continue;
                }
                let backend = dom.backend_node_id;
                if let Some(index) = self.ax_index.get(&backend) {
                    let node = &self.nodes[*index];
                    if !node.ax.ignored
                        && target
                            .role
                            .as_ref()
                            .is_none_or(|role| ax_text(&node.ax.role) == role)
                        && target.name.as_ref().is_none_or(|name| {
                            text_matches(ax_text(&node.ax.name), name, target.exact == Some(true))
                        })
                    {
                        matches.push(TargetElement::resolve(page, backend).await?);
                    }
                }
            }
            matches
        };
        if let Some(container) = within {
            matches.retain(|element| {
                let mut backend = element.backend_node_id;
                loop {
                    if backend == container {
                        return true;
                    }
                    let Some(parent) = self.parents.get(&backend) else {
                        return false;
                    };
                    backend = *parent;
                }
            });
        }
        if let Some(nth) = target.nth {
            matches = matches.into_iter().nth(nth as usize).into_iter().collect();
        }
        Ok(matches)
    }

    /// Match browser label/rendered-text locators once per captured frame.
    async fn text_targets(
        &self,
        page: &Page,
        target: &BrowserTarget,
    ) -> Result<Vec<TargetElement>> {
        use chromiumoxide::cdp::js_protocol::runtime::{
            CallArgument, CallFunctionOnParams, DeepSerializedValueType, SerializationOptions,
            SerializationOptionsSerialization,
        };
        let (kind, expected) = match &target.label {
            Some(label) => ("label", label),
            None => ("text", target.text_match.as_ref().expect("text locator")),
        };
        let mut backends = Vec::new();
        for frame in &self.documents {
            let context = page
                .frame_execution_context(frame.clone())
                .await?
                .ok_or(BrowserError::StaleReference)?;
            let result = page
                .evaluate_function(
                    CallFunctionOnParams::builder()
                        .function_declaration(include_str!("locator.js"))
                        .execution_context_id(context)
                        .object_group(super::element::OBJECT_GROUP)
                        .arguments(
                            vec![
                                json!(kind),
                                json!(expected),
                                json!(target.exact == Some(true)),
                            ]
                            .into_iter()
                            .map(|value| CallArgument::builder().value(value).build())
                            .collect::<Vec<_>>(),
                        )
                        .serialization_options(
                            SerializationOptions::builder()
                                .serialization(SerializationOptionsSerialization::Deep)
                                .max_depth(2)
                                .additional_parameters(json!({"maxNodeDepth": 0}))
                                .build()
                                .map_err(BrowserError::Configuration)?,
                        )
                        .build()
                        .map_err(BrowserError::Configuration)?,
                )
                .await?;
            let serialized = result
                .object()
                .deep_serialized_value
                .as_ref()
                .filter(|value| value.r#type == DeepSerializedValueType::Array)
                .and_then(|value| value.value.as_ref())
                .ok_or_else(|| {
                    BrowserError::InvalidResult(
                        "locator did not return a serialized node array".into(),
                    )
                })?;
            let nodes: Vec<SerializedNode> = serde_json::from_value(serialized.clone())
                .map_err(|error| BrowserError::InvalidResult(format!("locator nodes: {error}")))?;
            for SerializedNode::Node { backend_node_id } in nodes {
                if !self.dom_index.contains_key(&backend_node_id) {
                    return Err(BrowserError::StaleReference);
                }
                backends.push(backend_node_id);
            }
        }
        backends.sort_by_key(|backend| self.dom_index[backend]);
        let mut matches = Vec::with_capacity(backends.len());
        for backend in backends {
            matches.push(TargetElement::resolve(page, backend).await?);
        }
        Ok(matches)
    }

    /// Removal satisfies hidden/detached only inside the reference's original document.
    pub async fn resolve_wait(
        &self,
        page: &Page,
        target: &BrowserTarget,
        refs: &References,
    ) -> Result<Vec<TargetElement>> {
        if let Some(id) = &target.r#ref {
            let reference = refs.by_id.get(id).ok_or(BrowserError::StaleReference)?;
            if self.loaders.get(&reference.frame) != Some(&reference.loader) {
                return Err(BrowserError::StaleReference);
            }
            if !self
                .dom_index
                .get(&reference.backend)
                .is_some_and(|index| self.dom[*index].1 == reference.frame)
            {
                return Ok(Vec::new());
            }
        }
        self.resolve(page, target, refs).await
    }

    pub fn describe_elements(
        &self,
        elements: &[TargetElement],
        refs: &mut References,
        limit: usize,
    ) -> Result<Vec<BrowserNode>> {
        elements
            .iter()
            .take(limit)
            .map(|element| {
                let index = self.ax_index.get(&element.backend_node_id);
                if let Some(index) = index {
                    self.describe(*index, 0, refs)
                } else {
                    let index = self
                        .dom_index
                        .get(&element.backend_node_id)
                        .ok_or(BrowserError::StaleReference)?;
                    let frame = &self.dom[*index].1;
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
        let reference = refs.by_id.get(id).ok_or(BrowserError::StaleReference)?;
        if self.loaders.get(&reference.frame) != Some(&reference.loader) {
            return Err(BrowserError::StaleReference);
        }
        self.dom_index
            .get(&reference.backend)
            .filter(|index| self.dom[**index].1 == reference.frame)
            .map(|_| reference.backend)
            .ok_or(BrowserError::StaleReference)
    }
}

// Blink includes additional DOM metadata in deep serialization. Validate the
// node discriminator and backend identity; the locator does not use that metadata.
#[derive(Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
enum SerializedNode {
    Node {
        #[serde(rename = "backendNodeId")]
        backend_node_id: BackendNodeId,
    },
}

impl References {
    pub fn invalidate(&mut self) {
        self.by_id.clear();
        self.by_node.clear();
    }

    fn issue(&mut self, reference: Reference) -> Result<String> {
        if let Some(id) = self.by_node.get(&reference) {
            return Ok(id.clone());
        }
        if self.by_id.len() >= 10_000 {
            return Err(BrowserError::Configuration(
                "browser reference limit reached for this document".into(),
            ));
        }
        let id = super::handles::fresh("e")?;
        self.by_id.insert(id.clone(), reference.clone());
        self.by_node.insert(reference, id.clone());
        Ok(id)
    }
}

/// Preserve the observed browser hierarchy in the public inspect result.
pub(super) fn hierarchy(nodes: Vec<BrowserNode>) -> Result<serde_json::Value> {
    let mut roots: Vec<(u64, serde_json::Value)> = Vec::new();
    for node in nodes.into_iter().rev() {
        let depth = node.depth;
        let mut value = serde_json::to_value(node)
            .map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
        let object = value.as_object_mut().expect("browser node is an object");
        object.remove("depth");
        object.remove("bounds");
        let mut children = Vec::new();
        while roots
            .last()
            .is_some_and(|(child_depth, _)| *child_depth > depth)
        {
            children.push(roots.pop().expect("child exists").1);
        }
        if !children.is_empty() {
            object.insert("children".into(), json!(children));
        }
        roots.push((depth, value));
    }
    Ok(json!(
        roots
            .into_iter()
            .rev()
            .map(|(_, node)| node)
            .collect::<Vec<_>>()
    ))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ax_values_omit_unsupported_types_without_losing_nodes() {
        for (kind, value, expected) in [
            ("boolean", json!(false), None),
            ("valueUndefined", json!(null), None),
            ("string", json!("input value"), Some(json!("input value"))),
            ("number", json!(42), Some(json!(42.0))),
        ] {
            let ax = serde_json::from_value(json!({
                "nodeId": "1", "ignored": false,
                "role": {"type": "role", "value": "textbox"},
                "value": {"type": kind, "value": value},
            }))
            .unwrap();
            let observation = Observation {
                nodes: vec![Node {
                    ax,
                    frame: FrameId::new("frame"),
                    loader: LoaderId::new("loader"),
                    children: Vec::new(),
                }],
                order: vec![(0, 0)],
                ..Observation::default()
            };
            let (nodes, truncated) = observation.tree(&mut References::default(), 100).unwrap();
            assert!(!truncated);
            assert_eq!(nodes.len(), 1);
            assert_eq!(
                serde_json::to_value(&nodes[0]).unwrap().get("value"),
                expected.as_ref()
            );
        }
    }
}
