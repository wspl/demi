//! The page observers of watched tabs (`live-view.md` § Input): an
//! isolated world in each of the tab's documents reports the cursors, the
//! native form controls and copied text, and applies the viewer's choices.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use chromiumoxide::cdp::{
    browser_protocol::{
        dom::{DescribeNodeParams, GetBoxModelParams, GetFrameOwnerParams, SetFileInputFilesParams},
        emulation::SetFocusEmulationEnabledParams,
        page::{
            AddScriptToEvaluateOnNewDocumentParams, CreateIsolatedWorldParams, EventFrameNavigated,
            FrameId,
        },
        target::{EventAttachedToTarget, EventDetachedFromTarget, TargetId},
    },
    js_protocol::runtime::{
        AddBindingParams, EvaluateParams, EventBindingCalled, EventExecutionContextCreated,
        EventExecutionContextDestroyed, EventExecutionContextsCleared, ReleaseObjectParams,
        RemoteObject,
    },
};
use chromiumoxide::{Page, listeners::EventStreamError};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{broadcast, mpsc, watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::driver::operation::{BrowserError, Result};
use crate::tabs::tab::BrowserTab;
use demi_command_package_browser_protocol::live::{
    ControlToken, CursorRegion, LiveControl, MAX_CURSOR_REGIONS,
};

/// The isolated world's name, shared by its script and its binding.
const WORLD: &str = "demi-live";
const BINDING: &str = "demiLiveReport";
const SOURCE: &str = include_str!("observer.js");
/// How long the place of a frame in the tab may take to read.
const PLACE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
/// How long a new document gets to take the viewer's pointer.
const POINTER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// What the observer of one tab last reported.
pub(crate) struct Observed {
    pub controls: watch::Sender<Vec<LiveControl>>,
    /// The CSS cursor under the pointer, and whether it is over editable text.
    pub cursor: watch::Sender<(String, bool)>,
    /// Where on the visible page each cursor applies, from every frame that
    /// reports them: the top document's first, then its frames'.
    pub regions: watch::Sender<Vec<CursorRegion>>,
    /// The viewer's pointer in the tab, as a viewer last moved it there; a
    /// new top document resolves its cursor at it before any pointer event.
    pub pointer: watch::Sender<Option<(f64, f64)>>,
    /// Text the page copied.
    pub copies: broadcast::Sender<String>,
    /// Counts the tab's main documents: a new one ends the input held in the
    /// old one.
    pub documents: watch::Sender<u64>,
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum Report {
    Cursor { cursor: String, editable: bool },
    /// The regions of the reporting frame, in its own viewport's CSS pixels.
    Cursors { regions: Vec<CursorRegion> },
    Controls { controls: Vec<LiveControl> },
    Copy { text: String },
}

/// A document of the tab whose observer reports: the renderer session it
/// lives in, as the tab's page or a cross-site frame's own target, and its
/// world there.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct World {
    /// The session's target id.
    page: String,
    context: i64,
}

/// What the tab's observed pages tell the observer's owner.
enum Heard {
    Report { world: World, payload: String },
    /// An observer's world started in a frame of `page`.
    Created { world: World, unique: String, frame: FrameId },
    Destroyed { unique: String },
    /// Every world of `page` ended, as its documents were replaced.
    Cleared { page: TargetId },
    /// The tab's top document was replaced.
    Navigated,
    /// A cross-site frame of `parent` got its own renderer session.
    Attached { parent: TargetId, child: TargetId },
    /// A renderer session embedded in the tab ended, as its frame went away.
    Detached { session: String },
}

/// One renderer session of the tab: the tab's page, or a cross-site
/// frame's, with the session it is embedded in.
struct Observing {
    page: Page,
    parent: Option<TargetId>,
}

/// Starts the observer of `tab` in every frame of it, cross-site frames in
/// their own processes included; it ends with the tab.
pub(crate) async fn start(tab: &BrowserTab, tasks: &TaskTracker) -> Result<Arc<Observed>> {
    let (heard, mut hearing) = mpsc::channel(64);
    listen(tab.page(), true, heard.clone(), tab.ended().clone(), tasks).await?;
    // The tab a viewer watches is the one in front, as the viewer's page
    // is in the user's browser.
    tab.page()
        .execute(SetFocusEmulationEnabledParams::new(true))
        .await?;
    // The observer reads what the page writes to a clipboard that is
    // the browser's own, as the agent's clipboard commands do.
    if crate::page::clipboard::unisolated().is_none() {
        crate::page::clipboard::grant(tab.browser()).await?;
    }
    let observed = Arc::new(Observed {
        controls: watch::channel(Vec::new()).0,
        cursor: watch::channel(("default".to_owned(), false)).0,
        regions: watch::channel(Vec::new()).0,
        pointer: watch::channel(None).0,
        copies: broadcast::channel(4).0,
        documents: watch::channel(0).0,
    });
    let mut pages = HashMap::from([(
        tab.page().target_id().clone(),
        Observing {
            page: tab.page().clone(),
            parent: None,
        },
    )]);
    // The cross-site frames already there; later ones are heard of as they attach.
    for document in crate::driver::frames::capture(tab.page()).await?.frames {
        let child = document.page.target_id().clone();
        let Some(parent) = document.parent_page.map(|page| page.target_id().clone()) else {
            continue;
        };
        if pages.contains_key(&child) || child == parent {
            continue;
        }
        if listen(&document.page, false, heard.clone(), tab.ended().clone(), tasks).await.is_ok() {
            pages.insert(child, Observing { page: document.page, parent: Some(parent) });
        }
    }
    let reporting = observed.clone();
    let ended = tab.ended().clone();
    let observing = tab.clone();
    let spawner = tasks.clone();
    tasks.spawn(async move {
        let mut frames = FrameRegions::default();
        loop {
            let heard_now = tokio::select! {
                _ = ended.cancelled() => break,
                heard_now = hearing.recv() => match heard_now {
                    Some(heard_now) => heard_now,
                    None => break,
                },
            };
            match heard_now {
                Heard::Report { world, payload } => {
                    // The page cannot reach this world; a report it cannot
                    // parse is the observer's own defect.
                    match serde_json::from_str::<Report>(&payload) {
                        Ok(Report::Cursor { cursor, editable }) => {
                            // Each move reports its frame's cursor; the viewers
                            // hear only a change of the tab's one cursor.
                            let reported = (cursor, editable);
                            reporting.cursor.send_if_modified(|current| {
                                if *current == reported {
                                    return false;
                                }
                                *current = reported;
                                true
                            });
                        }
                        Ok(Report::Cursors { regions }) => {
                            frames.regions.insert(world, regions);
                            reporting.regions.send_replace(frames.merged(&pages, &observing).await);
                        }
                        Ok(Report::Controls { controls }) => {
                            reporting.controls.send_replace(controls);
                        }
                        Ok(Report::Copy { text }) => {
                            let _unwatched = reporting.copies.send(text);
                        }
                        Err(error) => tracing::warn!("live view observer report: {error}"),
                    }
                }
                Heard::Created { world, unique, frame } => {
                    frames.contexts.insert(unique, world.clone());
                    frames.documents.insert(world, frame);
                }
                Heard::Destroyed { unique } => {
                    if frames.remove(&unique) {
                        reporting.regions.send_replace(frames.merged(&pages, &observing).await);
                    }
                }
                Heard::Cleared { page } => {
                    frames.regions.retain(|world, _| world.page != page.as_ref());
                    reporting.regions.send_replace(frames.merged(&pages, &observing).await);
                }
                Heard::Navigated => {
                    // A new document has no controls or cursors yet, nor the
                    // old one's cross-site frames; its cursor at the still
                    // pointer replaces the old one's once it resolves it there.
                    reporting.controls.send_replace(Vec::new());
                    pages.retain(|_, observing| observing.parent.is_none());
                    frames.regions.clear();
                    reporting.regions.send_replace(Vec::new());
                    reporting.documents.send_modify(|documents| *documents += 1);
                    let pointer = *reporting.pointer.borrow();
                    if let Some((x, y)) = pointer {
                        let tab = observing.clone();
                        spawner.spawn(async move {
                            let expression = format!("globalThis.demiLive.pointer({x}, {y})");
                            let told = call(&tab, &expression, true);
                            // A document replaced again meanwhile waits for its own pointer events.
                            tokio::select! {
                                _ = tab.ended().cancelled() => {}
                                _told = tokio::time::timeout(POINTER_TIMEOUT, told) => {}
                            }
                        });
                    }
                }
                Heard::Detached { session } => {
                    let gone: Vec<TargetId> = pages
                        .iter()
                        .filter(|(_, observing)| observing.page.session_id().as_ref() == session)
                        .map(|(id, _)| id.clone())
                        .collect();
                    for id in gone {
                        pages.remove(&id);
                        frames.regions.retain(|world, _| world.page != id.as_ref());
                        reporting.regions.send_replace(frames.merged(&pages, &observing).await);
                    }
                }
                Heard::Attached { parent, child } => {
                    if pages.contains_key(&child) {
                        continue;
                    }
                    let Some(embedder) = pages.get(&parent) else {
                        continue;
                    };
                    // A frame that went away before its session settled has nothing to observe.
                    let Ok(Some(page)) = embedder.page.related_page(child.clone()).await else {
                        continue;
                    };
                    if listen(&page, false, heard.clone(), observing.ended().clone(), &spawner)
                        .await
                        .is_ok()
                    {
                        pages.insert(child, Observing { page, parent: Some(parent) });
                    }
                }
            }
        }
    });
    Ok(observed)
}

/// Puts the observer into every document of `page`'s renderer session, the
/// ones there now and the ones to come, and hands what it reports to
/// `heard` until `ended`. The tab's own page also tells of its top
/// document's replacement; each page tells of the cross-site frames that
/// attach to it.
async fn listen(
    page: &Page,
    tab_page: bool,
    heard: mpsc::Sender<Heard>,
    ended: CancellationToken,
    tasks: &TaskTracker,
) -> Result<()> {
    let mut reports = page.event_listener::<EventBindingCalled>().await?;
    let mut navigations = page.event_listener::<EventFrameNavigated>().await?;
    let mut created = page.event_listener::<EventExecutionContextCreated>().await?;
    let mut destroyed = page.event_listener::<EventExecutionContextDestroyed>().await?;
    let mut cleared = page.event_listener::<EventExecutionContextsCleared>().await?;
    let mut attached = page.event_listener::<EventAttachedToTarget>().await?;
    let mut detached = page.event_listener::<EventDetachedFromTarget>().await?;
    page.execute(
        AddBindingParams::builder()
            .name(BINDING)
            .execution_context_name(WORLD)
            .build()
            .map_err(BrowserError::Configuration)?,
    )
    .await?;
    page.execute(
        AddScriptToEvaluateOnNewDocumentParams::builder()
            .source(SOURCE)
            .world_name(WORLD)
            .run_immediately(true)
            .build()
            .map_err(BrowserError::Configuration)?,
    )
    .await?;
    let id = page.target_id().clone();
    tasks.spawn(async move {
        loop {
            let next = tokio::select! {
                _ = ended.cancelled() => break,
                report = reports.next() => match report {
                    Some(Ok(report)) if report.name == BINDING => Some(Heard::Report {
                        world: World { page: id.as_ref().to_owned(), context: *report.execution_context_id.inner() },
                        payload: report.payload.clone(),
                    }),
                    // Reports are snapshots; a newer one follows.
                    Some(Ok(_)) | Some(Err(EventStreamError::Lagged(_))) => None,
                    Some(Err(_)) | None => break,
                },
                navigation = navigations.next() => match navigation {
                    Some(Ok(navigation)) if tab_page && navigation.frame.parent_id.is_none() => Some(Heard::Navigated),
                    Some(Ok(_)) | Some(Err(EventStreamError::Lagged(_))) => None,
                    Some(Err(_)) | None => break,
                },
                world = created.next() => match world {
                    Some(Ok(world)) if world.context.name == WORLD => {
                        let frame = world
                            .context
                            .aux_data
                            .as_ref()
                            .and_then(|data| data.get("frameId"))
                            .and_then(Value::as_str)
                            .map(|frame| FrameId::new(frame.to_owned()));
                        frame.map(|frame| Heard::Created {
                            world: World { page: id.as_ref().to_owned(), context: *world.context.id.inner() },
                            unique: world.context.unique_id.clone(),
                            frame,
                        })
                    }
                    Some(Ok(_)) | Some(Err(EventStreamError::Lagged(_))) => None,
                    Some(Err(_)) | None => break,
                },
                gone = destroyed.next() => match gone {
                    Some(Ok(gone)) => Some(Heard::Destroyed { unique: gone.execution_context_unique_id.clone() }),
                    Some(Err(EventStreamError::Lagged(_))) => None,
                    Some(Err(_)) | None => break,
                },
                all = cleared.next() => match all {
                    Some(Ok(_)) => Some(Heard::Cleared { page: id.clone() }),
                    Some(Err(EventStreamError::Lagged(_))) => None,
                    Some(Err(_)) | None => break,
                },
                child = attached.next() => match child {
                    Some(Ok(child)) if child.target_info.r#type == "iframe" => Some(Heard::Attached {
                        parent: id.clone(),
                        child: child.target_info.target_id.clone(),
                    }),
                    Some(Ok(_)) | Some(Err(EventStreamError::Lagged(_))) => None,
                    Some(Err(_)) | None => break,
                },
                child = detached.next() => match child {
                    Some(Ok(child)) => Some(Heard::Detached { session: child.session_id.as_ref().to_owned() }),
                    Some(Err(EventStreamError::Lagged(_))) => None,
                    Some(Err(_)) | None => break,
                },
            };
            // The owner ended with the tab.
            if let Some(next) = next
                && heard.send(next).await.is_err()
            {
                break;
            }
        }
    });
    Ok(())
}

/// The cursor regions each frame's observer last reported, in its own
/// viewport's CSS pixels, by its world.
#[derive(Default)]
struct FrameRegions {
    regions: BTreeMap<World, Vec<CursorRegion>>,
    /// The frame each world is in.
    documents: HashMap<World, FrameId>,
    /// The worlds by the unique ids their end is reported with.
    contexts: HashMap<String, World>,
}

impl FrameRegions {
    /// Forgets the frame whose world `unique` ended with its document;
    /// whether it had reported regions.
    fn remove(&mut self, unique: &str) -> bool {
        let Some(world) = self.contexts.remove(unique) else {
            return false;
        };
        self.documents.remove(&world);
        self.regions.remove(&world).is_some()
    }

    /// Every frame's regions in the tab's CSS pixels: the top document's
    /// first, then its frames', which lie over it. A frame whose place in
    /// the tab cannot be read, as one going away, counts for nothing.
    async fn merged(&self, pages: &HashMap<TargetId, Observing>, tab: &BrowserTab) -> Vec<CursorRegion> {
        let root = tab.page().mainframe().await.ok().flatten();
        let mut top = Vec::new();
        let mut framed = Vec::new();
        for (world, regions) in &self.regions {
            let Some(frame) = self.documents.get(world) else {
                continue;
            };
            let is_top = world.page == tab.page().target_id().as_ref() && Some(frame) == root.as_ref();
            let Some((x, y)) = frame_origin(pages, &TargetId::new(world.page.clone()), frame).await else {
                continue;
            };
            let placed = regions.iter().map(|region| CursorRegion {
                x: region.x + x,
                y: region.y + y,
                ..region.clone()
            });
            if is_top {
                top.extend(placed);
            } else {
                framed.extend(placed);
            }
        }
        top.extend(framed);
        top.truncate(MAX_CURSOR_REGIONS);
        top
    }
}

/// Where the viewport of `frame`, a document of `page`'s renderer session,
/// stands in the tab, in CSS pixels: the content boxes of the frames that
/// embed it, added up to the tab's own page.
async fn frame_origin(
    pages: &HashMap<TargetId, Observing>,
    page: &TargetId,
    frame: &FrameId,
) -> Option<(f64, f64)> {
    let (mut x, mut y) = (0.0, 0.0);
    let mut page = page.clone();
    let mut frame = frame.clone();
    loop {
        let observing = pages.get(&page)?;
        let root = observing.page.mainframe().await.ok()??;
        // A frame inside the session's own document: its box there.
        if frame != root {
            let (left, top) = owner_box(&observing.page, &frame).await?;
            x += left;
            y += top;
        }
        let Some(parent) = &observing.parent else {
            return Some((x, y));
        };
        // The session's own document is a cross-site frame: its box in the session that embeds it.
        let embedder = pages.get(parent)?;
        let (left, top) = owner_box(&embedder.page, &root).await?;
        x += left;
        y += top;
        frame = embedder.page.mainframe().await.ok()??;
        page = parent.clone();
    }
}

/// The top-left corner of the content box of the element that embeds
/// `frame` in `page`'s session, in that session's top viewport. A session
/// that went away without its detachment reaching the observer yet would
/// answer only at the request timeout, so it is given a moment and then
/// counts as gone.
async fn owner_box(page: &Page, frame: &FrameId) -> Option<(f64, f64)> {
    let owner = tokio::time::timeout(PLACE_TIMEOUT, page.execute(GetFrameOwnerParams::new(frame.clone())))
        .await
        .ok()?
        .ok()?
        .result
        .backend_node_id;
    let boxed = page.execute(GetBoxModelParams::builder().backend_node_id(owner).build());
    let model = tokio::time::timeout(PLACE_TIMEOUT, boxed)
        .await
        .ok()?
        .ok()?
        .result
        .model;
    let content = model.content.inner();
    Some((*content.first()?, *content.get(1)?))
}

/// Runs `expression` in the observer's world of the tab's main document, and
/// waits for the promise it returns, if any.
async fn call(tab: &BrowserTab, expression: &str, by_value: bool) -> Result<RemoteObject> {
    let frame = tab
        .page()
        .mainframe()
        .await?
        .ok_or(BrowserError::TabNotFound)?;
    let world = tab
        .page()
        .execute(
            CreateIsolatedWorldParams::builder()
                .frame_id(frame)
                .world_name(WORLD)
                .build()
                .map_err(BrowserError::Configuration)?,
        )
        .await?
        .result
        .execution_context_id;
    // The source starts the observer if this document's world lacks it.
    let evaluated = tab
        .page()
        .execute(
            EvaluateParams::builder()
                .expression(format!("{SOURCE}\n{expression}"))
                .context_id(world)
                .await_promise(true)
                .return_by_value(by_value)
                .build()
                .map_err(BrowserError::Configuration)?,
        )
        .await?
        .result;
    if let Some(exception) = evaluated.exception_details {
        return Err(BrowserError::InvalidResult(exception.text));
    }
    Ok(evaluated.result)
}

/// Applies a viewer's choice in a select, date, time, color or suggestion
/// control to the revision the viewer saw.
pub(crate) async fn choose(
    tab: &BrowserTab,
    token: &ControlToken,
    revision: u64,
    value: &str,
    indices: &[u32],
) -> Result<bool> {
    let message = json!({"token": token, "revision": revision, "value": value, "indices": indices});
    let result = call(tab, &format!("globalThis.demiLive.commit({message})"), true).await?;
    Ok(result.value == Some(Value::Bool(true)))
}

/// Attaches files the viewer chose to a file input, for the revision the
/// viewer saw.
pub(crate) async fn attach(
    tab: &BrowserTab,
    token: &ControlToken,
    revision: u64,
    files: Vec<String>,
) -> Result<bool> {
    let control = json!({"token": token, "revision": revision});
    let element = call(
        tab,
        &format!("globalThis.demiLive.element({control})"),
        false,
    )
    .await?;
    let Some(object) = element.object_id else {
        return Ok(false);
    };
    let attached = async {
        let node = tab
            .page()
            .execute(
                DescribeNodeParams::builder()
                    .object_id(object.clone())
                    .build(),
            )
            .await?
            .result
            .node;
        if node.local_name != "input" {
            return Ok(false);
        }
        tab.page()
            .execute(
                SetFileInputFilesParams::builder()
                    .files(files)
                    .backend_node_id(node.backend_node_id)
                    .build()
                    .map_err(BrowserError::Configuration)?,
            )
            .await?;
        call(
            tab,
            &format!("globalThis.demiLive.committed({control})"),
            true,
        )
        .await?;
        Ok(true)
    }
    .await;
    let _released = tab.page().execute(ReleaseObjectParams::new(object)).await;
    attached
}

/// Puts the viewer's clipboard on the browser's own clipboard, when the Host
/// keeps that apart from its user's clipboard; false when it does not.
pub(crate) async fn write_clipboard(tab: &BrowserTab, text: &str, html: &str) -> Result<bool> {
    if !crate::page::clipboard::capability(tab.page())
        .await?
        .available
    {
        return Ok(false);
    }
    crate::page::clipboard::grant(tab.browser()).await?;
    let clipboard = json!({"text": text, "html": html});
    let written = call(
        tab,
        &format!(
            r#"(async ({{ text, html }}) => {{
                const items = {{ 'text/plain': new Blob([text], {{ type: 'text/plain' }}) }};
                if (html) items['text/html'] = new Blob([html], {{ type: 'text/html' }});
                try {{
                    await navigator.clipboard.write([new ClipboardItem(items)]);
                    return true;
                }} catch {{
                    return false;
                }}
            }})({clipboard})"#
        ),
        true,
    )
    .await?;
    Ok(written.value == Some(Value::Bool(true)))
}
