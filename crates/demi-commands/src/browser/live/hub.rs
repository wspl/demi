//! One browser's live view (`live-view.md`): who watches which tab,
//! the streams viewers of the same tab share, and the viewports and the
//! screen the viewers decide.
//!
//! One owner task per environment keeps all of it and lays out the screen
//! and the watched tabs' viewports itself, so layouts never overlap and a
//! burst of changes costs one layout. Viewers reach it through their
//! membership; a membership's end is noticed without a message, so dropping
//! one always leaves.

use std::{collections::HashMap, future::Future, pin::Pin};

use futures_util::{FutureExt, StreamExt, stream::FuturesUnordered};
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::{
    sync::{CancellationToken, DropGuard},
    task::TaskTracker,
};

use super::{
    capture::CaptureChannel,
    stream::{StreamHandle, StreamView},
};
use crate::browser::{
    BrowserError, BrowserTab, Result,
    protocol::{BrowserViewport, TabId, ViewportMode},
    viewport::{PHONE, Screen, ratio_for, screen_ratio},
};

/// Requests waiting for the hub; a full queue holds back their viewers.
const REQUESTS: usize = 32;
/// Notices a viewer has not written to its page yet; past them the hub
/// drops more, having logged them.
const NOTICES: usize = 4;

/// A viewer's panel in CSS pixels, and its screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Panel {
    pub width: u32,
    pub height: u32,
    pub ratio: f64,
    pub screen_width: u32,
    pub screen_height: u32,
}

/// Something the hub could not do for a viewer, for its page.
pub(super) struct Notice {
    pub code: String,
    pub message: String,
}

enum Request {
    Join {
        notices: mpsc::Sender<Notice>,
        left: CancellationToken,
        reply: oneshot::Sender<u64>,
    },
    Panel {
        viewer: u64,
        panel: Panel,
    },
    Operated {
        viewer: u64,
    },
    Watch {
        viewer: u64,
        tab: Option<BrowserTab>,
        reply: oneshot::Sender<Option<StreamView>>,
    },
    Mode {
        viewer: u64,
        tab: BrowserTab,
        mode: ViewportMode,
        reply: oneshot::Sender<Result<()>>,
    },
}

/// The way to one browser's live view hub.
pub(crate) struct Hub {
    requests: mpsc::Sender<Request>,
    changes: watch::Sender<u64>,
}

impl Hub {
    /// Starts the hub of an environment; it ends with the environment.
    pub fn new(captures: CaptureChannel, tasks: TaskTracker, ended: CancellationToken) -> Self {
        let (requests, received) = mpsc::channel(REQUESTS);
        let owner = Owner {
            captures,
            tasks: tasks.clone(),
            order: 0,
            viewers: HashMap::new(),
            driver: None,
            streams: HashMap::new(),
            screen: None,
            due: false,
        };
        tasks.spawn(owner.run(received, ended));
        Self {
            requests,
            changes: watch::channel(0).0,
        }
    }

    /// The notifier tabs share to tell viewers that what they show changed.
    pub fn changes(&self) -> watch::Sender<u64> {
        self.changes.clone()
    }

    pub(super) fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    /// Joins the hub; what it cannot do for the viewer arrives as notices.
    pub(super) async fn join(&self) -> Result<(Membership, mpsc::Receiver<Notice>)> {
        let left = CancellationToken::new();
        // Leaves even if this future is dropped before it returns.
        let leave = left.clone().drop_guard();
        let (notices, noticed) = mpsc::channel(NOTICES);
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::Join {
                notices,
                left,
                reply,
            })
            .await
            .map_err(|_| BrowserError::Closed)?;
        let id = answer.await.map_err(|_| BrowserError::Closed)?;
        Ok((
            Membership {
                requests: self.requests.clone(),
                id,
                _leave: leave,
            },
            noticed,
        ))
    }
}

/// A viewer's place in the hub; dropping it leaves.
pub(super) struct Membership {
    requests: mpsc::Sender<Request>,
    id: u64,
    _leave: DropGuard,
}

impl Membership {
    /// The hub ends with the environment, whose view ends with it too.
    async fn tell(&self, request: Request) {
        let _ended = self.requests.send(request).await;
    }

    pub async fn panel(&self, panel: Panel) {
        self.tell(Request::Panel {
            viewer: self.id,
            panel,
        })
        .await;
    }

    /// The viewer pressed a button or a key, turned the wheel, pasted or
    /// chose: it now decides the screen and the tabs it watches.
    pub async fn operated(&self) {
        self.tell(Request::Operated { viewer: self.id }).await;
    }

    /// Watches `tab`, or nothing; the tab's pictures arrive on the view.
    pub async fn watch(&self, tab: Option<&BrowserTab>) -> Option<StreamView> {
        let (reply, answer) = oneshot::channel();
        self.tell(Request::Watch {
            viewer: self.id,
            tab: tab.cloned(),
            reply,
        })
        .await;
        answer.await.ok().flatten()
    }

    /// Puts `tab` in Web or Mobile mode, sized for whoever decides it.
    pub async fn mode(&self, tab: &BrowserTab, mode: ViewportMode) -> Result<()> {
        let (reply, answer) = oneshot::channel();
        self.tell(Request::Mode {
            viewer: self.id,
            tab: tab.clone(),
            mode,
            reply,
        })
        .await;
        answer.await.map_err(|_| BrowserError::Closed)?
    }
}

struct Viewer {
    panel: Option<Panel>,
    watching: Option<BrowserTab>,
    /// When the viewer last operated, in hub order; 0 if never.
    operated: u64,
    joined: u64,
    notices: mpsc::Sender<Notice>,
}

type Departure = Pin<Box<dyn Future<Output = u64> + Send>>;

struct Owner {
    captures: CaptureChannel,
    tasks: TaskTracker,
    order: u64,
    viewers: HashMap<u64, Viewer>,
    /// The viewer that operated most recently, whose screen the browser's is.
    driver: Option<u64>,
    streams: HashMap<TabId, StreamHandle>,
    /// The screen last set.
    screen: Option<Screen>,
    /// A layout is due once the requests at hand are handled.
    due: bool,
}

impl Owner {
    async fn run(mut self, mut requests: mpsc::Receiver<Request>, ended: CancellationToken) {
        // Each viewer's departure: the owner is shared with the layouts it
        // awaits, which these futures could not be.
        let mut departures = FuturesUnordered::<Departure>::new();
        loop {
            tokio::select! {
                biased;
                _ = ended.cancelled() => break,
                Some(viewer) = departures.next() => self.leave(viewer),
                request = requests.recv() => match request {
                    Some(request) => self.request(request, &mut departures).await,
                    None => break,
                },
            }
            // What arrived meanwhile goes into the same layout.
            while let Ok(request) = requests.try_recv() {
                self.request(request, &mut departures).await;
            }
            while let Some(Some(viewer)) = departures.next().now_or_never() {
                self.leave(viewer);
            }
            if self.due && !ended.is_cancelled() {
                self.due = false;
                self.layout().await;
            }
        }
    }

    async fn request(&mut self, request: Request, departures: &mut FuturesUnordered<Departure>) {
        match request {
            Request::Join {
                notices,
                left,
                reply,
            } => {
                self.order += 1;
                let id = self.order;
                self.viewers.insert(
                    id,
                    Viewer {
                        panel: None,
                        watching: None,
                        operated: 0,
                        joined: id,
                        notices,
                    },
                );
                departures.push(Box::pin(async move {
                    left.cancelled_owned().await;
                    id
                }));
                // A viewer that left before it heard its ID has left anyway.
                let _left = reply.send(id);
            }
            Request::Panel { viewer, panel } => {
                // A viewer that left decides nothing.
                let Some(entry) = self.viewers.get_mut(&viewer) else {
                    return;
                };
                entry.panel = Some(panel);
                // Without a driver, the first viewer to show its screen decides.
                if self
                    .driver
                    .is_none_or(|driver| !self.viewers.contains_key(&driver))
                {
                    self.driver = Some(viewer);
                }
                self.due = true;
            }
            Request::Operated { viewer } => {
                self.order += 1;
                let order = self.order;
                let others = self.viewers.len() > 1;
                let Some(entry) = self.viewers.get_mut(&viewer) else {
                    return;
                };
                let decided = entry.operated;
                entry.operated = order;
                let driving = self.driver == Some(viewer);
                self.driver = Some(viewer);
                // Another viewer's panel may have decided until now.
                if !driving || (decided == 0 && others) {
                    self.due = true;
                }
            }
            Request::Watch { viewer, tab, reply } => {
                // A viewer that left watches nothing: its request, sent as it
                // left, would give a stream a member nobody takes away.
                let Some(entry) = self.viewers.get_mut(&viewer) else {
                    return;
                };
                let previous = std::mem::replace(&mut entry.watching, tab.clone());
                if let Some(previous) = previous {
                    self.leave_stream(previous.id(), viewer);
                }
                let view = tab.map(|tab| {
                    let captures = &self.captures;
                    let tasks = &self.tasks;
                    self.streams
                        .entry(tab.id().clone())
                        .or_insert_with(|| StreamHandle::start(tab, captures.clone(), tasks))
                        .join(viewer)
                });
                self.due = true;
                // A viewer that left no longer needs its view.
                let _left = reply.send(view);
            }
            Request::Mode {
                viewer,
                tab,
                mode,
                reply,
            } => {
                let result = self.mode(viewer, &tab, mode).await;
                let _left = reply.send(result);
            }
        }
    }

    fn leave(&mut self, viewer: u64) {
        if let Some(entry) = self.viewers.remove(&viewer)
            && let Some(tab) = entry.watching
        {
            self.leave_stream(tab.id(), viewer);
        }
        // The screen keeps its ratio until another viewer operates.
        if self.driver == Some(viewer) {
            self.driver = None;
        }
        self.due = true;
    }

    /// Takes `viewer` off `tab`'s stream; a stream nobody watches ends.
    fn leave_stream(&mut self, tab: &TabId, viewer: u64) {
        if let Some(stream) = self.streams.get(tab) {
            stream.leave(viewer);
            if stream.is_empty() {
                self.streams.remove(tab);
            }
        }
    }

    /// The panel that decides a watched tab's size: that of the viewer who
    /// watches it and operated most recently.
    fn decider(&self, tab: &TabId) -> Option<Panel> {
        self.viewers
            .values()
            .filter(|viewer| {
                viewer
                    .watching
                    .as_ref()
                    .is_some_and(|watched| watched.id() == tab)
            })
            .filter_map(|viewer| viewer.panel.map(|panel| (viewer, panel)))
            .max_by_key(|(viewer, _)| (viewer.operated, std::cmp::Reverse(viewer.joined)))
            .map(|(_, panel)| panel)
    }

    /// The screen follows the driver; a watched Web or Mobile tab takes its
    /// decider's panel and ratio. What cannot be applied is logged and told
    /// to the viewers it was for (`live-view.md` § Opening a view).
    async fn layout(&mut self) {
        let desired = self
            .driver
            .and_then(|driver| self.viewers.get(&driver))
            .and_then(|viewer| viewer.panel)
            .map(|panel| Screen {
                width: panel.screen_width,
                height: panel.screen_height,
                ratio: screen_ratio(panel.ratio),
            });
        let mut tabs: Vec<(BrowserTab, Panel)> = Vec::new();
        for viewer in self.viewers.values() {
            if let Some(tab) = &viewer.watching
                && !tabs.iter().any(|(listed, _)| listed.id() == tab.id())
                && let Some(panel) = self.decider(tab.id())
            {
                tabs.push((tab.clone(), panel));
            }
        }
        if let Some(desired) = desired
            && self.screen != Some(desired)
            && let Some((tab, _)) = tabs.first()
        {
            match tab.update_screen(desired).await {
                Ok(()) => self.screen = Some(desired),
                Err(error) => {
                    tracing::warn!("live view screen: {error}");
                    let driver = self.driver;
                    self.notify(|viewer, _| Some(viewer) == driver, &error);
                }
            }
        }
        for (tab, panel) in tabs {
            if let Err(error) = fit(&tab, tab.viewport().mode, panel).await
                && !tab.ended.is_cancelled()
            {
                tracing::warn!("live view viewport of {}: {error}", tab.id());
                self.notify(
                    |_, entry| {
                        entry
                            .watching
                            .as_ref()
                            .is_some_and(|watched| watched.id() == tab.id())
                    },
                    &error,
                );
            }
        }
    }

    /// Puts `tab` in Web or Mobile mode, sized for whoever decides it.
    async fn mode(&self, viewer: u64, tab: &BrowserTab, mode: ViewportMode) -> Result<()> {
        let panel = self
            .decider(tab.id())
            .or_else(|| self.viewers.get(&viewer).and_then(|viewer| viewer.panel));
        match panel {
            Some(panel) => fit(tab, mode, panel).await,
            // Nobody has shown a panel: the tab keeps its last Web size.
            None => {
                let web = tab.web_viewport();
                let (width, height) = match mode {
                    ViewportMode::Mobile => PHONE,
                    _ => (web.width, web.height),
                };
                tab.set_viewport(BrowserViewport {
                    mode,
                    width,
                    height,
                    device_pixel_ratio: web.device_pixel_ratio,
                })
                .await
            }
        }
    }

    /// Tells the viewers `chosen` picks about `error`; a viewer that is not
    /// keeping up loses the notice, which the log has.
    fn notify(&self, chosen: impl Fn(u64, &Viewer) -> bool, error: &BrowserError) {
        for (id, viewer) in &self.viewers {
            if chosen(*id, viewer) {
                let _behind = viewer.notices.try_send(Notice {
                    code: error.code().to_string(),
                    message: error.to_string(),
                });
            }
        }
    }
}

/// Gives a Web or Mobile tab the viewport `panel` decides.
async fn fit(tab: &BrowserTab, mode: ViewportMode, panel: Panel) -> Result<()> {
    let (width, height) = match mode {
        ViewportMode::Web => (panel.width, panel.height),
        ViewportMode::Mobile => PHONE,
        ViewportMode::Custom => return Ok(()),
    };
    let viewport = BrowserViewport {
        mode,
        width,
        height,
        device_pixel_ratio: ratio_for(panel.ratio, width, height),
    };
    if tab.viewport() != viewport {
        tab.set_viewport(viewport).await?;
    }
    Ok(())
}
