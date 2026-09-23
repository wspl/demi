//! Chrome's accessibility tree supplies names, roles, hierarchy and semantic targets.

use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;

use chromiumoxide::{
    Page,
    cdp::browser_protocol::{
        accessibility::{AxNode, AxValue, GetFullAxTreeParams},
        dom::{BackendNodeId, GetFrameOwnerParams, Node as DomNode},
        network::LoaderId,
        page::FrameId,
        target::TargetId,
    },
};

use super::{
    BrowserError, Result,
    element::TargetElement,
    protocol::{BrowserNode, BrowserTarget, BrowserTreeNode, NodeRef},
};

pub(super) type DomIdentity = (TargetId, BackendNodeId);

#[derive(Clone, PartialEq, Eq, Hash)]
struct Reference {
    backend: BackendNodeId,
    frame: FrameId,
    loader: LoaderId,
}

#[derive(Default)]
pub(super) struct References {
    by_id: HashMap<NodeRef, Reference>,
    by_node: HashMap<Reference, NodeRef>,
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
    order: Vec<(usize, usize)>,
    dom: Vec<(DomNode, FrameId)>,
    loaders: HashMap<FrameId, LoaderId>,
    dom_index: HashMap<DomIdentity, usize>,
    ax_index: HashMap<DomIdentity, usize>,
    parents: HashMap<DomIdentity, DomIdentity>,
    documents: Vec<FrameId>,
    pages: HashMap<FrameId, Page>,
    inaccessible: HashMap<FrameId, String>,
    scope: Option<DomIdentity>,
}

impl Observation {
    /// Join each frame's AX tree at its iframe element, preserving document identity.
    pub async fn capture(page: &Page, refs: &mut References) -> Result<Self> {
        let snapshot = super::frames::capture(page).await?;
        let main_frame = snapshot.main;
        let mut documents_by_session = snapshot.documents;
        let mut pages = HashMap::new();
        let mut inaccessible = HashMap::new();
        let mut nodes = Vec::new();
        let mut ax_index = HashMap::new();
        let mut roots = HashMap::new();
        let mut owners = Vec::new();
        let mut loaders = HashMap::new();
        for document in snapshot.frames {
            let frame = document.frame;
            let frame_page = document.page;
            if let Some(parent_page) = document.parent_page {
                let owner = parent_page
                    .execute(GetFrameOwnerParams::new(frame.id.clone()))
                    .await?
                    .result
                    .backend_node_id;
                owners.push((frame.id.clone(), (parent_page.target_id().clone(), owner)));
            }
            pages.insert(frame.id.clone(), frame_page.clone());
            loaders.insert(frame.id.clone(), frame.loader_id.clone());
            let ax = match frame_page
                .execute(
                    GetFullAxTreeParams::builder()
                        .frame_id(frame.id.clone())
                        .build(),
                )
                .await
            {
                Ok(result) => result.result.nodes,
                Err(chromiumoxide::error::CdpError::Chrome(error)) => {
                    inaccessible.insert(frame.id.clone(), error.to_string());
                    Vec::new()
                }
                Err(error) => return Err(error.into()),
            };
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
                    ax_index
                        .entry((frame_page.target_id().clone(), backend))
                        .or_insert(nodes.len());
                }
                nodes.push(Node {
                    ax: node,
                    frame: frame.id.clone(),
                    loader: frame.loader_id.clone(),
                    children,
                });
            }
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
                pending.push((*child, depth + usize::from(visible)));
            }
        }
        let (document, _) = documents_by_session
            .remove(page.target_id())
            .ok_or(BrowserError::StaleReference)?;
        let mut dom = Vec::new();
        let mut dom_index = HashMap::new();
        let mut parents = HashMap::new();
        let mut documents = Vec::new();
        let mut pending = vec![(document, main_frame, None)];
        while let Some((mut node, frame, parent)) = pending.pop() {
            let backend = (pages[&frame].target_id().clone(), node.backend_node_id);
            if let Some(parent) = parent {
                parents.insert(backend.clone(), parent);
            }
            if let Some(document) = node.content_document.take() {
                let child_frame = node.frame_id.clone().ok_or_else(|| {
                    BrowserError::InvalidResult("frame document has no frame ID".into())
                })?;
                pending.push((*document, child_frame, Some(backend.clone())));
            }
            if let Some(child_frame) = &node.frame_id
                && let Some(child_page) = pages.get(child_frame)
                && let Some((document, document_frame)) =
                    documents_by_session.remove(child_page.target_id())
            {
                pending.push((document, document_frame, Some(backend.clone())));
            }
            let mut children = node.children.take().unwrap_or_default();
            children.extend(node.shadow_roots.take().unwrap_or_default());
            pending.extend(
                children
                    .into_iter()
                    .rev()
                    .map(|child| (child, frame.clone(), Some(backend.clone()))),
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
            pages,
            inaccessible,
            scope: None,
        })
    }

    pub fn tree(&self, refs: &mut References, limit: usize) -> Result<(Vec<BrowserNode>, bool)> {
        let selected: Vec<_> = self
            .order
            .iter()
            .filter(|(index, _)| {
                let node = &self.nodes[*index];
                self.scope.as_ref().is_none_or(|scope| {
                    node.ax.backend_dom_node_id.is_some_and(|backend| {
                        self.descendant(
                            (self.pages[&node.frame].target_id().clone(), backend),
                            scope.clone(),
                            true,
                        )
                    })
                })
            })
            .collect();
        let nodes = selected
            .iter()
            .take(limit)
            .map(|(index, depth)| self.describe(*index, *depth, refs))
            .collect::<Result<_>>()?;
        Ok((nodes, selected.len() > limit))
    }

    /// Enter explicit browser frame references in order and validate an optional container.
    fn target_scope(
        &self,
        within: Option<&NodeRef>,
        frames: Option<&[NodeRef]>,
        refs: &References,
    ) -> Result<Option<DomIdentity>> {
        let mut scope = None;
        let mut expected_frame = None;
        for reference in frames.into_iter().flatten() {
            let key = self.reference(reference, refs)?;
            let (node, parent_frame) = &self.dom[self.dom_index[&key]];
            if !matches!(node.local_name.as_str(), "iframe" | "frame") {
                return Err(BrowserError::Configuration(
                    "--frame must reference an iframe or frame element".into(),
                ));
            }
            if expected_frame
                .as_ref()
                .is_some_and(|expected| expected != parent_frame)
            {
                return Err(BrowserError::Configuration(
                    "--frame references must enter frames from outermost to innermost".into(),
                ));
            }
            let child_frame = node.frame_id.as_ref().ok_or_else(|| {
                BrowserError::UnsupportedCapability(
                    "frame element has no captured child frame ID".into(),
                )
            })?;
            if !self.loaders.contains_key(child_frame)
                || self.inaccessible.contains_key(child_frame)
            {
                return Err(BrowserError::UnsupportedCapability(format!(
                    "frame document is inaccessible: {}",
                    self.inaccessible
                        .get(child_frame)
                        .map_or("frame absent from captured tree", String::as_str)
                )));
            }
            expected_frame = Some(child_frame.clone());
            scope = Some(key);
        }
        if let Some(reference) = within {
            let container = self.reference(reference, refs)?;
            if scope
                .as_ref()
                .is_some_and(|scope| !self.descendant(container.clone(), scope.clone(), false))
            {
                return Err(BrowserError::Configuration(
                    "--within is outside the selected frame".into(),
                ));
            }
            scope = Some(container);
        }
        Ok(scope)
    }

    pub(super) fn restrict(
        &mut self,
        within: Option<&NodeRef>,
        frames: Option<&[NodeRef]>,
        refs: &References,
    ) -> Result<()> {
        self.scope = self.target_scope(within, frames, refs)?;
        Ok(())
    }

    /// Describe the same captured DOM with references, omitting script/style implementation text.
    pub fn dom_tree(
        &self,
        refs: &mut References,
        limit: usize,
    ) -> Result<(Vec<BrowserTreeNode>, bool)> {
        let mut nodes = Vec::new();
        let mut truncated = false;
        for (dom, frame) in &self.dom {
            if self.scope.as_ref().is_some_and(|scope| {
                !self.descendant(
                    (self.pages[frame].target_id().clone(), dom.backend_node_id),
                    scope.clone(),
                    true,
                )
            }) {
                continue;
            }
            if dom.node_type != 1
                || matches!(dom.local_name.as_str(), "script" | "style" | "noscript")
            {
                continue;
            }
            if nodes.len() == limit {
                truncated = true;
                break;
            }
            let mut depth = 0;
            let mut ancestor = (self.pages[frame].target_id().clone(), dom.backend_node_id);
            while let Some(parent) = self.parents.get(&ancestor) {
                if self
                    .dom_index
                    .get(parent)
                    .is_some_and(|index| self.dom[*index].0.node_type == 1)
                {
                    depth += 1;
                }
                ancestor = parent.clone();
            }
            let loader = self
                .loaders
                .get(frame)
                .ok_or(BrowserError::StaleReference)?;
            let reference = refs.issue(Reference {
                backend: dom.backend_node_id,
                frame: frame.clone(),
                loader: loader.clone(),
            })?;
            let mut node = BrowserTreeNode {
                r#ref: Some(reference),
                tag: Some(dom.local_name.clone()),
                children: Some(Vec::new()),
                ..BrowserTreeNode::default()
            };
            if let Some(index) = self
                .ax_index
                .get(&(self.pages[frame].target_id().clone(), dom.backend_node_id))
            {
                let described = self.describe(*index, 0, refs)?;
                node.role = Some(described.role);
                node.name = Some(described.name);
                node.states = Some(described.states);
                node.value = described.value;
            }
            nodes.push((depth, node));
        }
        Ok((nest(nodes), truncated))
    }

    fn describe(&self, index: usize, depth: usize, refs: &mut References) -> Result<BrowserNode> {
        let node = &self.nodes[index];
        let reference = match node.ax.backend_dom_node_id {
            Some(backend) => Some(refs.issue(Reference {
                backend,
                frame: node.frame.clone(),
                loader: node.loader.clone(),
            })?),
            None => None,
        };
        let protected = node.ax.backend_dom_node_id.is_some_and(|backend| {
            self.protected_key(&(self.pages[&node.frame].target_id().clone(), backend))
        });
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
        if node.ax.backend_dom_node_id.is_some_and(|backend| {
            self.dom_index
                .get(&(self.pages[&node.frame].target_id().clone(), backend))
                .and_then(|index| self.dom[*index].0.frame_id.as_ref())
                .is_some_and(|frame| self.inaccessible.contains_key(frame))
        }) {
            states.push("inaccessible".into());
        }
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
    fn protected_key(&self, backend: &DomIdentity) -> bool {
        self.dom_index.get(backend).is_some_and(|index| {
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

    pub(super) fn accessible(&self, element: &TargetElement) -> bool {
        self.ax_index
            .get(&element.identity())
            .is_some_and(|index| !self.nodes[*index].ax.ignored)
    }

    pub fn protected(&self, element: &TargetElement) -> bool {
        self.protected_key(&element.identity())
    }

    async fn resolve_identity(&self, identity: &DomIdentity) -> Result<TargetElement> {
        let index = self
            .dom_index
            .get(identity)
            .ok_or(BrowserError::StaleReference)?;
        let mut element =
            TargetElement::resolve(&self.pages[&self.dom[*index].1], identity.1).await?;
        let mut parent = identity;
        while let Some(key) = self.parents.get(parent) {
            let index = self
                .dom_index
                .get(key)
                .ok_or(BrowserError::StaleReference)?;
            let (node, frame) = &self.dom[*index];
            if matches!(node.local_name.as_str(), "iframe" | "frame") {
                element
                    .frame_chain
                    .push((self.pages[frame].clone(), node.backend_node_id));
            }
            parent = key;
        }
        Ok(element)
    }

    /// Resolve fresh semantic/CSS matches against the same observation used for output.
    pub async fn resolve(
        &self,
        page: &Page,
        target: &BrowserTarget,
        refs: &References,
    ) -> Result<Vec<TargetElement>> {
        validate_target(target)?;
        let within = self.target_scope(target.within.as_ref(), target.frame.as_deref(), refs)?;
        let mut matches = if let Some(reference) = &target.r#ref {
            vec![
                self.resolve_identity(&self.reference(reference, refs)?)
                    .await?,
            ]
        } else if target.css.is_some()
            || target.placeholder.is_some()
            || target.test_id.is_some()
            || target.label.is_some()
            || target.text_match.is_some()
            || target.text_pattern.is_some()
        {
            self.text_targets(page, target).await?
        } else {
            let names = self
                .nodes
                .iter()
                .map(|node| ax_text(&node.ax.name))
                .collect::<Vec<_>>();
            let name_matches = match &target.name_pattern {
                Some(pattern) => Some(super::query::pattern_matches(page, pattern, &names).await?),
                None => None,
            };
            let mut matches = Vec::new();
            for (dom, frame) in &self.dom {
                if dom.node_type != 1 {
                    continue;
                }
                let backend = (self.pages[frame].target_id().clone(), dom.backend_node_id);
                if let Some(index) = self.ax_index.get(&backend) {
                    let node = &self.nodes[*index];
                    if !node.ax.ignored
                        && name_matches.as_ref().is_none_or(|matches| matches[*index])
                        && target
                            .role
                            .as_ref()
                            .is_none_or(|role| ax_text(&node.ax.role).eq_ignore_ascii_case(role))
                        && target.name.as_ref().is_none_or(|name| {
                            text_matches(ax_text(&node.ax.name), name, target.exact == Some(true))
                        })
                    {
                        matches.push(self.resolve_identity(&backend).await?);
                    }
                }
            }
            matches
        };
        if let Some(container) = within {
            matches.retain(|element| self.descendant(element.identity(), container.clone(), true));
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
        let (kind, expected) = if let Some(css) = &target.css {
            ("css", css)
        } else if let Some(label) = &target.label {
            ("label", label)
        } else if let Some(placeholder) = &target.placeholder {
            ("placeholder", placeholder)
        } else if let Some(test_id) = &target.test_id {
            ("test-id", test_id)
        } else if let Some(pattern) = &target.text_pattern {
            ("text-pattern", pattern)
        } else {
            ("text", target.text_match.as_ref().expect("text locator"))
        };
        if kind == "text-pattern" {
            super::query::pattern_matches(page, expected, &[]).await?;
        }
        if kind == "css" {
            let error: String = page.evaluate_expression(format!("(() => {{ try {{ document.querySelector({}); return ''; }} catch (error) {{ return error.message; }} }})()", json!(expected))).await?.into_value().map_err(|error| BrowserError::InvalidResult(error.to_string()))?;
            if !error.is_empty() {
                return Err(BrowserError::Configuration(error));
            }
        }
        let mut backends = Vec::new();
        for frame in &self.documents {
            backends.extend(
                self.scan_frame(
                    frame,
                    include_str!("locator.js"),
                    vec![
                        json!(kind),
                        json!(expected),
                        json!(target.exact == Some(true)),
                    ],
                )
                .await?,
            );
        }
        backends.sort_by_key(|backend| self.dom_index[backend]);
        let mut matches = Vec::with_capacity(backends.len());
        for backend in backends {
            matches.push(self.resolve_identity(&backend).await?);
        }
        Ok(matches)
    }

    /// Resolve only browser nodes returned by one composed-tree evaluation per frame.
    async fn scan_frame(
        &self,
        frame: &FrameId,
        script: &str,
        args: Vec<serde_json::Value>,
    ) -> Result<Vec<DomIdentity>> {
        use chromiumoxide::cdp::js_protocol::runtime::{
            CallArgument, CallFunctionOnParams, DeepSerializedValueType, SerializationOptions,
            SerializationOptionsSerialization,
        };
        let page = &self.pages[frame];
        let context = page
            .frame_execution_context(frame.clone())
            .await?
            .ok_or(BrowserError::StaleReference)?;
        let result = page
            .evaluate_function(
                CallFunctionOnParams::builder()
                    .function_declaration(script)
                    .execution_context_id(context)
                    .object_group(super::element::OBJECT_GROUP)
                    .arguments(
                        args.into_iter()
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
                BrowserError::InvalidResult("locator did not return a serialized node array".into())
            })?;
        let nodes: Vec<SerializedNode> = serde_json::from_value(serialized.clone())
            .map_err(|error| BrowserError::InvalidResult(format!("locator nodes: {error}")))?;
        nodes
            .into_iter()
            .map(|SerializedNode::Node { backend_node_id }| {
                let key = (page.target_id().clone(), backend_node_id);
                if self.dom_index.contains_key(&key) {
                    Ok(key)
                } else {
                    Err(BrowserError::StaleReference)
                }
            })
            .collect()
    }

    /// Probe the same captured frame documents and join candidates to their AX references.
    pub(super) async fn probe_targets(
        &self,
        x: f64,
        y: f64,
        include_ordinary: bool,
    ) -> Result<Vec<TargetElement>> {
        let mut matches = Vec::new();
        for frame in &self.documents {
            let document = self
                .dom
                .iter()
                .find(|(node, owner)| node.node_type == 9 && owner == frame)
                .ok_or(BrowserError::StaleReference)?;
            let key = (
                self.pages[frame].target_id().clone(),
                document.0.backend_node_id,
            );
            let document = self.resolve_identity(&key).await?;
            let mut local = [x, y];
            for (page, backend) in &document.frame_chain {
                let owner = TargetElement::resolve(page, *backend).await?;
                let offset = super::element::frame_offset(&owner).await?;
                local[0] -= offset[0];
                local[1] -= offset[1];
            }
            for key in self
                .scan_frame(
                    frame,
                    include_str!("probe.js"),
                    vec![json!(local[0]), json!(local[1]), json!(include_ordinary)],
                )
                .await?
            {
                matches.push(self.resolve_identity(&key).await?);
            }
        }
        self.dom_order(&mut matches);
        Ok(matches)
    }

    pub(super) fn descendant(
        &self,
        mut backend: DomIdentity,
        container: DomIdentity,
        inclusive: bool,
    ) -> bool {
        if !inclusive {
            let Some(parent) = self.parents.get(&backend) else {
                return false;
            };
            backend = parent.clone();
        }
        loop {
            if backend == container {
                return true;
            }
            let Some(parent) = self.parents.get(&backend) else {
                return false;
            };
            backend = parent.clone();
        }
    }

    pub(super) fn dom_order(&self, elements: &mut Vec<TargetElement>) {
        elements.sort_by_key(|element| self.dom_index.get(&element.identity()).copied());
        elements.dedup_by_key(|element| element.identity());
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
                .get(&(
                    self.pages[&reference.frame].target_id().clone(),
                    reference.backend,
                ))
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
                let index = self.ax_index.get(&element.identity());
                if let Some(index) = index {
                    self.describe(*index, 0, refs)
                } else {
                    let index = self
                        .dom_index
                        .get(&element.identity())
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

    pub(super) fn reference(&self, id: &NodeRef, refs: &References) -> Result<DomIdentity> {
        let reference = refs.by_id.get(id).ok_or(BrowserError::StaleReference)?;
        if self.loaders.get(&reference.frame) != Some(&reference.loader) {
            return Err(BrowserError::StaleReference);
        }
        self.dom_index
            .get(&(
                self.pages[&reference.frame].target_id().clone(),
                reference.backend,
            ))
            .filter(|index| self.dom[**index].1 == reference.frame)
            .map(|_| {
                (
                    self.pages[&reference.frame].target_id().clone(),
                    reference.backend,
                )
            })
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

    fn issue(&mut self, reference: Reference) -> Result<NodeRef> {
        if let Some(id) = self.by_node.get(&reference) {
            return Ok(id.clone());
        }
        if self.by_id.len() >= 10_000 {
            return Err(BrowserError::Configuration(
                "browser reference limit reached for this document".into(),
            ));
        }
        let id = NodeRef::from_random(super::handles::random()?);
        self.by_id.insert(id.clone(), reference.clone());
        self.by_node.insert(reference, id.clone());
        Ok(id)
    }
}

/// Preserve the observed browser hierarchy in the public inspect result.
pub(super) fn hierarchy(nodes: Vec<BrowserNode>) -> Vec<BrowserTreeNode> {
    nest(
        nodes
            .into_iter()
            .map(|node| {
                let tree = BrowserTreeNode {
                    r#ref: node.r#ref,
                    role: Some(node.role),
                    name: Some(node.name),
                    value: node.value,
                    tag: None,
                    states: Some(node.states),
                    children: None,
                };
                (node.depth, tree)
            })
            .collect(),
    )
}

/// Nest observed browser nodes in document order from their captured depths.
fn nest(nodes: Vec<(usize, BrowserTreeNode)>) -> Vec<BrowserTreeNode> {
    let mut roots: Vec<(usize, BrowserTreeNode)> = Vec::new();
    for (depth, mut node) in nodes.into_iter().rev() {
        let mut children = Vec::new();
        while roots
            .last()
            .is_some_and(|(child_depth, _)| *child_depth > depth)
        {
            children.push(roots.pop().expect("child exists").1);
        }
        if !children.is_empty() {
            node.children = Some(children);
        }
        roots.push((depth, node));
    }
    roots.into_iter().rev().map(|(_, node)| node).collect()
}

// Chrome AX values carry optional JSON; absent name/role means empty text.
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
    if count != 1
        || ((target.name.is_some() || target.name_pattern.is_some()) && target.role.is_none())
        || (target.name.is_some() && target.name_pattern.is_some())
        || (target.exact == Some(true)
            && (target.name_pattern.is_some() || target.text_pattern.is_some()))
    {
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
        target.text_pattern.is_some(),
        target.test_id.is_some(),
        target.css.is_some(),
    ]
    .into_iter()
    .filter(|value| *value)
    .count()
}

/// A locator can be sampled again; stored node and scope references cannot.
pub(super) fn can_resample(target: &BrowserTarget) -> bool {
    target.r#ref.is_none() && target.within.is_none() && target.frame.is_none()
}

/// URL and coordinate targets cannot silently ignore supplied element flags.
pub(super) fn has_target_flags(target: &BrowserTarget) -> bool {
    locator_count(target) > 0
        || target.name.is_some()
        || target.name_pattern.is_some()
        || target.frame.is_some()
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
