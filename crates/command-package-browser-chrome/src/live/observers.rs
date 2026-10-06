//! The page observers of watched tabs (`live-view.md` § Input): an
//! isolated world in each of the tab's documents reports the cursors, the
//! native form controls and copied text, and applies the viewer's choices.

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
};

use chromiumoxide::cdp::{
    browser_protocol::{
        dom::{DescribeNodeParams, SetFileInputFilesParams},
        emulation::SetFocusEmulationEnabledParams,
        page::{
            AddScriptToEvaluateOnNewDocumentParams, CreateIsolatedWorldParams, EventFrameNavigated,
        },
    },
    js_protocol::runtime::{
        AddBindingParams, EvaluateParams, EventBindingCalled, EventExecutionContextCreated,
        EventExecutionContextDestroyed, EventExecutionContextsCleared, ReleaseObjectParams,
        RemoteObject,
    },
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{broadcast, watch};
use tokio_util::task::TaskTracker;

use crate::driver::operation::{BrowserError, Result};
use crate::tabs::tab::BrowserTab;
use demi_command_package_browser_protocol::live::{
    ControlToken, CursorRegion, LiveControl, MAX_CURSOR_REGIONS,
};

/// The isolated world's name, shared by its script and its binding.
const WORLD: &str = "demi-live";
const BINDING: &str = "demiLiveReport";
const SOURCE: &str = include_str!("observer.js");
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
    Cursor {
        cursor: String,
        editable: bool,
    },
    Cursors {
        top: bool,
        regions: Vec<CursorRegion>,
    },
    Controls { controls: Vec<LiveControl> },
    Copy { text: String },
}

/// Starts the observer of `tab`; it ends with the tab.
pub(crate) async fn start(tab: &BrowserTab, tasks: &TaskTracker) -> Result<Arc<Observed>> {
    let mut reports = tab.page().event_listener::<EventBindingCalled>().await?;
    let mut navigations = tab.page().event_listener::<EventFrameNavigated>().await?;
    let mut created = tab
        .page()
        .event_listener::<EventExecutionContextCreated>()
        .await?;
    let mut destroyed = tab
        .page()
        .event_listener::<EventExecutionContextDestroyed>()
        .await?;
    let mut cleared = tab
        .page()
        .event_listener::<EventExecutionContextsCleared>()
        .await?;
    tab.page()
        .execute(
            AddBindingParams::builder()
                .name(BINDING)
                .execution_context_name(WORLD)
                .build()
                .map_err(BrowserError::Configuration)?,
        )
        .await?;
    tab.page()
        .execute(
            AddScriptToEvaluateOnNewDocumentParams::builder()
                .source(SOURCE)
                .world_name(WORLD)
                .run_immediately(true)
                .build()
                .map_err(BrowserError::Configuration)?,
        )
        .await?;
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
    let reporting = observed.clone();
    let ended = tab.ended().clone();
    let observing = tab.clone();
    let calls = tasks.clone();
    tasks.spawn(async move {
        // Each frame's regions by the execution context of its observer.
        let mut frames = FrameRegions::default();
        loop {
            tokio::select! {
                _ = ended.cancelled() => break,
                report = reports.next() => {
                    let report = match report {
                        Some(Ok(report)) => report,
                        // Reports are snapshots; a newer one follows.
                        Some(Err(chromiumoxide::listeners::EventStreamError::Lagged(_))) => continue,
                        Some(Err(_)) | None => break,
                    };
                    if report.name != BINDING {
                        continue;
                    }
                    // The page cannot reach this world; a report it
                    // cannot parse is the observer's own defect.
                    match serde_json::from_str::<Report>(&report.payload) {
                        Ok(Report::Cursor { cursor, editable }) => {
                            reporting.cursor.send_replace((cursor, editable));
                        }
                        Ok(Report::Cursors { top, regions }) => {
                            frames.report(*report.execution_context_id.inner(), top, regions);
                            reporting.regions.send_replace(frames.merged());
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
                navigation = navigations.next() => {
                    let navigation = match navigation {
                        Some(Ok(navigation)) => navigation,
                        Some(Err(chromiumoxide::listeners::EventStreamError::Lagged(_))) => continue,
                        Some(Err(_)) | None => break,
                    };
                    if navigation.frame.parent_id.is_none() {
                        // A new document has no controls or cursors yet; its
                        // cursor at the still pointer replaces the old one's
                        // once it resolves it there.
                        reporting.controls.send_replace(Vec::new());
                        frames.clear();
                        reporting.regions.send_replace(Vec::new());
                        reporting.documents.send_modify(|documents| *documents += 1);
                        let pointer = *reporting.pointer.borrow();
                        if let Some((x, y)) = pointer {
                            let tab = observing.clone();
                            calls.spawn(async move {
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
                }
                world = created.next() => {
                    let world = match world {
                        Some(Ok(world)) => world,
                        Some(Err(chromiumoxide::listeners::EventStreamError::Lagged(_))) => continue,
                        Some(Err(_)) | None => break,
                    };
                    if world.context.name == WORLD {
                        frames.contexts.insert(world.context.unique_id.clone(), *world.context.id.inner());
                    }
                }
                gone = destroyed.next() => {
                    let gone = match gone {
                        Some(Ok(gone)) => gone,
                        Some(Err(chromiumoxide::listeners::EventStreamError::Lagged(_))) => continue,
                        Some(Err(_)) | None => break,
                    };
                    if frames.remove(&gone.execution_context_unique_id) {
                        reporting.regions.send_replace(frames.merged());
                    }
                }
                all = cleared.next() => {
                    match all {
                        Some(Ok(_)) => {}
                        Some(Err(chromiumoxide::listeners::EventStreamError::Lagged(_))) => continue,
                        Some(Err(_)) | None => break,
                    }
                    frames.clear();
                    reporting.regions.send_replace(Vec::new());
                }
            }
        }
    });
    Ok(observed)
}

/// The cursor regions each frame's observer last reported, by the
/// execution context of its world.
#[derive(Default)]
struct FrameRegions {
    top: Option<(i64, Vec<CursorRegion>)>,
    frames: BTreeMap<i64, Vec<CursorRegion>>,
    /// The observers' worlds by the unique ids their end is reported with.
    contexts: HashMap<String, i64>,
}

impl FrameRegions {
    fn report(&mut self, context: i64, top: bool, regions: Vec<CursorRegion>) {
        if top {
            self.top = Some((context, regions));
        } else {
            self.frames.insert(context, regions);
        }
    }

    /// Forgets every frame's regions, as a new top document does; the
    /// worlds keep their ids.
    fn clear(&mut self) {
        self.top = None;
        self.frames.clear();
    }

    /// Forgets the frame whose world `unique` ended with its document;
    /// whether it had reported regions.
    fn remove(&mut self, unique: &str) -> bool {
        let Some(context) = self.contexts.remove(unique) else {
            return false;
        };
        if self.top.as_ref().is_some_and(|(top, _)| *top == context) {
            self.top = None;
            return true;
        }
        self.frames.remove(&context).is_some()
    }

    /// The top document's regions, then its frames', which lie over it.
    fn merged(&self) -> Vec<CursorRegion> {
        self.top
            .iter()
            .map(|(_, regions)| regions)
            .chain(self.frames.values())
            .flatten()
            .take(MAX_CURSOR_REGIONS)
            .cloned()
            .collect()
    }
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
