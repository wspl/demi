//! An action's target resolved in the tab's main document alone, without
//! reading the whole page: a reference by its node, a CSS selector or another
//! DOM locator by one scan of the document, a role by the accessibility tree's
//! own query. Each answers the same matches as the whole-page observation
//! (`observation.rs`) on such a page.
//!
//! The observation answers what the main document alone cannot: a target
//! scoped by `--within` or `--frame`, a reference into a frame, a page that
//! embeds frames, whose documents a locator also searches, a role the tree's
//! query finds nothing for, as one written in another case than Chrome's,
//! and a role with `--nth`, which counts in document order where the tree's
//! query answers in tree order.

use std::collections::HashMap;

use chromiumoxide::{
    Page,
    cdp::browser_protocol::{
        accessibility::{AxNode, GetPartialAxTreeParams, QueryAxTreeParams},
        dom::{BackendNodeId, GetDocumentParams},
        network::LoaderId,
        page::{FrameId, GetFrameTreeParams},
        target::{FilterEntry, GetTargetsParams, TargetFilter},
    },
};
use serde_json::json;

use crate::driver::operation::{BrowserError, Result};
use crate::page::{
    element::{TargetElement, call},
    observation::{ax_text, check_locator, dom_locator, scan, text_matches, validate_target},
    protocol::{BrowserTarget, ResolvedElement},
};
use crate::tabs::session::{Reference, References};

/// A target resolved in the main document, which names its matches.
pub(crate) struct Direct {
    frame: FrameId,
    loader: LoaderId,
    /// The role and name of each match, when the query read them.
    names: HashMap<BackendNodeId, (String, String)>,
}

/// The target's matches in the main document, or none when only the
/// whole-page observation can answer it.
pub(crate) async fn resolve(
    page: &Page,
    target: &BrowserTarget,
    refs: &References,
) -> Result<Option<(Direct, Vec<TargetElement>)>> {
    if target.within.is_some() || target.frame.is_some() {
        return Ok(None);
    }
    validate_target(target)?;
    if target.role.is_some() && target.nth.is_some() {
        return Ok(None);
    }
    let main = page
        .execute(GetFrameTreeParams {})
        .await?
        .result
        .frame_tree;
    let mut direct = Direct {
        frame: main.frame.id,
        loader: main.frame.loader_id,
        names: HashMap::new(),
    };
    if let Some(id) = &target.r#ref {
        let reference = refs.get(id).ok_or(BrowserError::StaleReference)?;
        if reference.frame != direct.frame {
            return Ok(None);
        }
        if reference.loader != direct.loader {
            return Err(BrowserError::StaleReference);
        }
        let element = TargetElement::resolve(page, reference.backend)
            .await
            .map_err(|error| match error {
                BrowserError::Cdp(_) => BrowserError::StaleReference,
                error => error,
            })?;
        // A node removed from the document, or moved into a frame's, is no
        // longer the one the reference names.
        let present: bool = call(
            page,
            &element,
            "function() { return this.isConnected && this.ownerDocument.defaultView === window.top; }",
            vec![],
        )
        .await?;
        if !present {
            return Err(BrowserError::StaleReference);
        }
        return Ok(Some((direct, nth(target, vec![element]))));
    }
    // An in-process frame is in the tree wherever its element is; a frame
    // another renderer draws is a target of its own.
    let embeds = main.child_frames.is_some_and(|frames| !frames.is_empty())
        || page
            .execute(
                GetTargetsParams::builder()
                    .filter(TargetFilter::new(vec![
                        FilterEntry::builder().r#type("iframe").build(),
                        FilterEntry::builder().exclude(true).build(),
                    ]))
                    .build(),
            )
            .await?
            .result
            .target_infos
            .iter()
            .any(|info| info.parent_id.as_ref() == Some(page.target_id()));
    if embeds {
        return Ok(None);
    }
    let backends = if let Some((kind, expected)) = dom_locator(target) {
        check_locator(page, kind, expected).await?;
        let context = page
            .frame_execution_context(direct.frame.clone())
            .await?
            .ok_or(BrowserError::StaleReference)?;
        scan(
            page,
            context,
            include_str!("locator.js"),
            vec![
                json!(kind),
                json!(expected),
                json!(target.exact == Some(true)),
            ],
        )
        .await?
    } else {
        let role = target.role.as_ref().expect("a validated role target");
        let nodes = query_role(page, role, target).await?;
        // The tree's query takes a role in Chrome's own case, `button` and
        // `DescriptionList` alike, while a target's role matches in any case:
        // a role the query does not know in the case given is the whole
        // page's to match.
        if nodes.is_empty() {
            return Ok(None);
        }
        let names = nodes
            .iter()
            .map(|node| ax_text(&node.name))
            .collect::<Vec<_>>();
        let name_matches = match &target.name_pattern {
            Some(pattern) => {
                Some(crate::page::query::pattern_matches(page, pattern, &names).await?)
            }
            None => None,
        };
        let mut backends = Vec::new();
        for (index, node) in nodes.iter().enumerate() {
            if let Some(backend) = node.backend_dom_node_id
                && !node.ignored
                && ax_text(&node.role).eq_ignore_ascii_case(role)
                && name_matches.as_ref().is_none_or(|matches| matches[index])
                && target.name.as_ref().is_none_or(|name| {
                    text_matches(names[index], name, target.exact == Some(true))
                })
            {
                direct
                    .names
                    .insert(backend, (ax_text(&node.role).into(), names[index].into()));
                backends.push(backend);
            }
        }
        backends
    };
    let mut matches = Vec::with_capacity(backends.len());
    for backend in backends {
        matches.push(TargetElement::resolve(page, backend).await?);
    }
    Ok(Some((direct, nth(target, matches))))
}

/// The match `--nth` picks, when the target names one, as the whole-page
/// observation picks it.
fn nth(target: &BrowserTarget, matches: Vec<TargetElement>) -> Vec<TargetElement> {
    match target.nth {
        Some(nth) => matches.into_iter().nth(nth).into_iter().collect(),
        None => matches,
    }
}

/// The accessibility tree's nodes with `role`, and with the target's name when
/// it is exact, in the main document.
async fn query_role(page: &Page, role: &str, target: &BrowserTarget) -> Result<Vec<AxNode>> {
    let document = page
        .execute(GetDocumentParams::builder().depth(0).build())
        .await?
        .result
        .root;
    let mut query = QueryAxTreeParams::builder()
        .backend_node_id(document.backend_node_id)
        .role(role);
    if target.exact == Some(true)
        && let Some(name) = &target.name
    {
        query = query.accessible_name(name);
    }
    Ok(page.execute(query.build()).await?.result.nodes)
}

impl Direct {
    /// What the page tree names `element`, with the reference that names it.
    pub(crate) async fn named(
        &self,
        page: &Page,
        element: &TargetElement,
        refs: &mut References,
    ) -> Result<ResolvedElement> {
        let (role, name) = match self.names.get(&element.backend_node_id) {
            Some(named) => named.clone(),
            None => tree_node(page, element.backend_node_id)
                .await?
                .map(|node| (ax_text(&node.role).into(), ax_text(&node.name).into()))
                .unwrap_or_default(),
        };
        Ok(ResolvedElement {
            r#ref: refs.issue(Reference {
                backend: element.backend_node_id,
                frame: self.frame.clone(),
                loader: self.loader.clone(),
            })?,
            role,
            name,
        })
    }
}

/// What the page tree names `element` of the main document, with the
/// reference that names it; none when the tree leaves the element out.
pub(crate) async fn named_in_main(
    page: &Page,
    element: &TargetElement,
    refs: &mut References,
) -> Result<Option<ResolvedElement>> {
    let frame = page
        .execute(GetFrameTreeParams {})
        .await?
        .result
        .frame_tree
        .frame;
    let Some(node) = tree_node(page, element.backend_node_id).await? else {
        return Ok(None);
    };
    if node.ignored {
        return Ok(None);
    }
    Ok(Some(ResolvedElement {
        r#ref: refs.issue(Reference {
            backend: element.backend_node_id,
            frame: frame.id,
            loader: frame.loader_id,
        })?,
        role: ax_text(&node.role).into(),
        name: ax_text(&node.name).into(),
    }))
}

/// The page tree's node of the DOM node `backend`, if the tree has one.
async fn tree_node(page: &Page, backend: BackendNodeId) -> Result<Option<AxNode>> {
    Ok(page
        .execute(
            GetPartialAxTreeParams::builder()
                .backend_node_id(backend)
                .fetch_relatives(false)
                .build(),
        )
        .await?
        .result
        .nodes
        .into_iter()
        .find(|node| node.backend_dom_node_id == Some(backend)))
}
