//! The page observers of watched tabs (`live-view.md` § Input): an
//! isolated world in each of the tab's documents reports the cursor, the
//! native form controls and copied text, and applies the viewer's choices.

use std::sync::Arc;

use chromiumoxide::cdp::{
    browser_protocol::{
        dom::{DescribeNodeParams, SetFileInputFilesParams},
        emulation::SetFocusEmulationEnabledParams,
        page::{
            AddScriptToEvaluateOnNewDocumentParams, CreateIsolatedWorldParams, EventFrameNavigated,
        },
    },
    js_protocol::runtime::{
        AddBindingParams, EvaluateParams, EventBindingCalled, ReleaseObjectParams, RemoteObject,
    },
};
use futures_util::StreamExt;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::sync::{broadcast, watch};
use tokio_util::task::TaskTracker;

use demi_builtin_protocol::live::{ControlToken, LiveControl};

use super::super::{BrowserError, BrowserTab, Result};

/// The isolated world's name, shared by its script and its binding.
const WORLD: &str = "demi-live";
const BINDING: &str = "demiLiveReport";
const SOURCE: &str = include_str!("observer.js");

/// What the observer of one tab last reported.
pub(crate) struct Observed {
    pub controls: watch::Sender<Vec<LiveControl>>,
    /// The CSS cursor under the pointer, and whether it is over editable text.
    pub cursor: watch::Sender<(String, bool)>,
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
    Controls {
        controls: Vec<LiveControl>,
    },
    Copy {
        text: String,
    },
}

/// The tab's observer, started the first time a viewer watches it.
pub(super) async fn observe(tab: &BrowserTab, tasks: &TaskTracker) -> Result<Arc<Observed>> {
    tab.state
        .observed
        .get_or_try_init(|| async {
            let mut reports = tab.page.event_listener::<EventBindingCalled>().await?;
            let mut navigations = tab.page.event_listener::<EventFrameNavigated>().await?;
            tab.page
                .execute(
                    AddBindingParams::builder()
                        .name(BINDING)
                        .execution_context_name(WORLD)
                        .build()
                        .map_err(BrowserError::Configuration)?,
                )
                .await?;
            tab.page
                .execute(
                    AddScriptToEvaluateOnNewDocumentParams::builder()
                        .source(SOURCE)
                        .world_name(WORLD)
                        .run_immediately(true)
                        .build()
                        .map_err(BrowserError::Configuration)?,
                )
                .await?;
            // The tab a viewer watches is the one in front, as in the
            // viewer's own browser.
            tab.page
                .execute(SetFocusEmulationEnabledParams::new(true))
                .await?;
            // The observer reads what the page writes to a clipboard that is
            // the browser's own, as the agent's clipboard commands do.
            if super::super::clipboard::unisolated().is_none() {
                super::super::clipboard::grant(&tab.browser).await?;
            }
            let observed = Arc::new(Observed {
                controls: watch::channel(Vec::new()).0,
                cursor: watch::channel(("default".to_owned(), false)).0,
                copies: broadcast::channel(4).0,
                documents: watch::channel(0).0,
            });
            let reporting = observed.clone();
            let ended = tab.ended.clone();
            tasks.spawn(async move {
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
                                Ok(Report::Controls { controls }) => {
                                    reporting.controls.send_replace(controls);
                                }
                                Ok(Report::Copy { text }) => {
                                    let _unwatched = reporting.copies.send(text);
                                }
                                Err(error) => eprintln!("live view observer report: {error}"),
                            }
                        }
                        navigation = navigations.next() => {
                            let navigation = match navigation {
                                Some(Ok(navigation)) => navigation,
                                Some(Err(chromiumoxide::listeners::EventStreamError::Lagged(_))) => continue,
                                Some(Err(_)) | None => break,
                            };
                            if navigation.frame.parent_id.is_none() {
                                // A new document has no controls yet, and the
                                // pointer's cursor is its own until it moves.
                                reporting.controls.send_replace(Vec::new());
                                reporting.cursor.send_replace(("default".to_owned(), false));
                                reporting.documents.send_modify(|documents| *documents += 1);
                            }
                        }
                    }
                }
            });
            Ok(observed)
        })
        .await
        .cloned()
}

/// Runs `expression` in the observer's world of the tab's main document, and
/// waits for the promise it returns, if any.
async fn call(tab: &BrowserTab, expression: &str, by_value: bool) -> Result<RemoteObject> {
    let frame = tab
        .page
        .mainframe()
        .await?
        .ok_or(BrowserError::TabNotFound)?;
    let world = tab
        .page
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
        .page
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
pub(super) async fn choose(
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
pub(super) async fn attach(
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
            .page
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
        tab.page
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
    let _released = tab.page.execute(ReleaseObjectParams::new(object)).await;
    attached
}

/// Puts the viewer's clipboard on the browser's own clipboard, when the Host
/// keeps that apart from its user's clipboard; false when it does not.
pub(super) async fn write_clipboard(tab: &BrowserTab, text: &str, html: &str) -> Result<bool> {
    if !super::super::clipboard::capability(&tab.page).await?.available {
        return Ok(false);
    }
    super::super::clipboard::grant(&tab.browser).await?;
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
