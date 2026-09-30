//! One frame inventory joins in-process documents and attached renderer sessions.
use super::Result;
use chromiumoxide::{
    Page,
    cdp::browser_protocol::{
        dom::{GetDocumentParams, Node},
        page::{Frame, FrameId, GetFrameTreeParams},
        target::TargetId,
    },
};
use std::collections::{HashMap, HashSet};

pub(super) struct Document {
    pub frame: Frame,
    pub page: Page,
    pub parent_page: Option<Page>,
}

pub(super) struct Snapshot {
    pub main: FrameId,
    pub frames: Vec<Document>,
    pub documents: HashMap<TargetId, (Node, FrameId)>,
}

/// Capture browser frame documents through each renderer that owns their DOM.
pub(super) async fn capture(page: &Page) -> Result<Snapshot> {
    let tree = page.execute(GetFrameTreeParams {}).await?.result.frame_tree;
    let main = tree.frame.id.clone();
    let mut pending = vec![(tree, page.clone(), None)];
    let mut frames = Vec::new();
    let mut documents = HashMap::new();
    let mut captured = HashSet::new();
    while let Some((mut tree, parent_page, parent)) = pending.pop() {
        if !captured.insert(tree.frame.id.clone()) {
            continue;
        }
        let related = if tree.frame.id == main {
            None
        } else {
            page.related_page(TargetId::new(tree.frame.id.as_ref()))
                .await?
        };
        let owner = related.as_ref().unwrap_or(&parent_page).clone();
        if related.is_some() {
            tree = owner
                .execute(GetFrameTreeParams {})
                .await?
                .result
                .frame_tree;
        }
        let frame = tree.frame;
        if !documents.contains_key(owner.target_id()) {
            let document = owner
                .execute(GetDocumentParams::builder().depth(-1).pierce(true).build())
                .await?
                .result
                .root;
            // Page.getFrameTree omits OOPIF children; their embedding DOM nodes
            // identify the attached sessions and each child's actual parent.
            let mut nodes = vec![(&document, frame.id.clone())];
            while let Some((node, containing_frame)) = nodes.pop() {
                if let Some(child_id) = &node.frame_id
                    && !captured.contains(child_id)
                    && let Some(child_page) =
                        page.related_page(TargetId::new(child_id.as_ref())).await?
                {
                    let child = child_page
                        .execute(GetFrameTreeParams {})
                        .await?
                        .result
                        .frame_tree;
                    pending.push((child, owner.clone(), Some(containing_frame.clone())));
                }
                if let Some(content) = &node.content_document {
                    nodes.push((
                        content,
                        node.frame_id
                            .clone()
                            .unwrap_or_else(|| containing_frame.clone()),
                    ));
                }
                nodes.extend(
                    node.children
                        .iter()
                        .flatten()
                        .chain(node.shadow_roots.iter().flatten())
                        .map(|child| (child, containing_frame.clone())),
                );
            }
            documents.insert(owner.target_id().clone(), (document, frame.id.clone()));
        }
        pending.extend(
            tree.child_frames
                .unwrap_or_default()
                .into_iter()
                .rev()
                .map(|tree| (tree, owner.clone(), Some(frame.id.clone()))),
        );
        frames.push(Document {
            frame,
            page: owner,
            parent_page: parent.map(|_| parent_page),
        });
    }
    Ok(Snapshot {
        main,
        frames,
        documents,
    })
}
