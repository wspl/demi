//! One browser's live view (`live-view.md`): who watches which tab,
//! the streams viewers of the same tab share, and the viewports and the
//! screen the viewers decide.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use tokio::sync::{mpsc, watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::{
    capture::Captures,
    stream::{self, Command, Event},
};
use crate::browser::{
    BrowserTab, Result,
    protocol::{BrowserViewport, TabId, ViewportMode},
    viewport::{PHONE, Screen, ratio_for, screen_ratio},
};

/// A viewer's panel in CSS pixels, and its screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Panel {
    pub width: u32,
    pub height: u32,
    pub ratio: f64,
    pub screen_width: u32,
    pub screen_height: u32,
}

struct Viewer {
    panel: Option<Panel>,
    watching: Option<BrowserTab>,
    /// When the viewer last operated, in hub order; 0 if never.
    operated: u64,
    joined: u64,
}

struct Stream {
    commands: mpsc::UnboundedSender<Command>,
    viewers: usize,
}

#[derive(Default)]
struct State {
    order: u64,
    viewers: HashMap<u64, Viewer>,
    /// The viewer that operated most recently, whose screen the browser's is.
    driver: Option<u64>,
    streams: HashMap<TabId, Stream>,
    /// A layout is running, and whether another is due after it.
    laying_out: bool,
    due: bool,
}

impl State {
    fn leave_stream(&mut self, tab: &TabId, viewer: u64) {
        if let Some(stream) = self.streams.get_mut(tab) {
            let _ended = stream.commands.send(Command::Leave { viewer });
            stream.viewers -= 1;
            if stream.viewers == 0 {
                self.streams.remove(tab);
            }
        }
    }

    fn command(&self, viewer: u64, command: Command) {
        let stream = self.viewers.get(&viewer).and_then(|viewer| {
            viewer
                .watching
                .as_ref()
                .and_then(|tab| self.streams.get(tab.id()))
        });
        if let Some(stream) = stream {
            let _ended = stream.commands.send(command);
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
}

pub(crate) struct Hub {
    pub(super) captures: Arc<Captures>,
    tasks: TaskTracker,
    ended: CancellationToken,
    changes: watch::Sender<u64>,
    state: Mutex<State>,
    /// The screen last set; held while the screen or a viewport changes.
    screen: tokio::sync::Mutex<Option<Screen>>,
}

impl Hub {
    pub fn new(captures: Arc<Captures>, tasks: TaskTracker, ended: CancellationToken) -> Self {
        Self {
            captures,
            tasks,
            ended,
            changes: watch::channel(0).0,
            state: Mutex::default(),
            screen: tokio::sync::Mutex::new(None),
        }
    }

    /// The notifier tabs share to tell viewers that what they show changed.
    pub fn changes(&self) -> watch::Sender<u64> {
        self.changes.clone()
    }

    /// Tabs opened, closed or changed.
    pub fn changed(&self) {
        self.changes.send_modify(|revision| *revision += 1);
    }

    pub(super) fn subscribe(&self) -> watch::Receiver<u64> {
        self.changes.subscribe()
    }

    pub(super) fn join(self: &Arc<Self>) -> Membership {
        let mut state = self.state.lock().expect("live lock poisoned");
        state.order += 1;
        let id = state.order;
        state.viewers.insert(
            id,
            Viewer {
                panel: None,
                watching: None,
                operated: 0,
                joined: id,
            },
        );
        Membership {
            hub: self.clone(),
            id,
        }
    }

    /// Lays out the screen and the watched tabs once the current layout, if
    /// any, finishes.
    fn relayout(self: &Arc<Self>) {
        let mut state = self.state.lock().expect("live lock poisoned");
        state.due = true;
        if state.laying_out {
            return;
        }
        state.laying_out = true;
        drop(state);
        let hub = self.clone();
        self.tasks.spawn(async move {
            loop {
                {
                    let mut state = hub.state.lock().expect("live lock poisoned");
                    if !state.due || hub.ended.is_cancelled() {
                        state.laying_out = false;
                        return;
                    }
                    state.due = false;
                }
                hub.layout().await;
            }
        });
    }

    /// The screen follows the driver; a watched Web or Mobile tab takes its
    /// decider's panel and ratio.
    async fn layout(&self) {
        let mut screen = self.screen.lock().await;
        let (desired, tabs) = {
            let state = self.state.lock().expect("live lock poisoned");
            let desired = state
                .driver
                .and_then(|driver| state.viewers.get(&driver))
                .and_then(|viewer| viewer.panel)
                .map(|panel| Screen {
                    width: panel.screen_width,
                    height: panel.screen_height,
                    ratio: screen_ratio(panel.ratio),
                });
            let mut tabs: Vec<(BrowserTab, Panel)> = Vec::new();
            for viewer in state.viewers.values() {
                if let Some(tab) = &viewer.watching
                    && !tabs.iter().any(|(listed, _)| listed.id() == tab.id())
                    && let Some(panel) = state.decider(tab.id())
                {
                    tabs.push((tab.clone(), panel));
                }
            }
            (desired, tabs)
        };
        if let Some(desired) = desired
            && *screen != Some(desired)
            && let Some((tab, _)) = tabs.first()
        {
            match tab.update_screen(desired).await {
                Ok(()) => *screen = Some(desired),
                Err(error) => eprintln!("live view screen: {error}"),
            }
        }
        for (tab, panel) in tabs {
            if let Err(error) = fit(&tab, tab.viewport().mode, panel).await
                && !tab.ended.is_cancelled()
            {
                eprintln!("live view viewport of {}: {error}", tab.id());
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

/// A viewer's place in the hub; dropping it leaves.
pub(super) struct Membership {
    hub: Arc<Hub>,
    id: u64,
}

impl Membership {
    pub fn panel(&self, panel: Panel) {
        let mut state = self.hub.state.lock().expect("live lock poisoned");
        if let Some(viewer) = state.viewers.get_mut(&self.id) {
            viewer.panel = Some(panel);
        }
        // Without a driver, the first viewer to show its screen decides.
        if state
            .driver
            .is_none_or(|driver| !state.viewers.contains_key(&driver))
        {
            state.driver = Some(self.id);
        }
        drop(state);
        self.hub.relayout();
    }

    /// The viewer pressed a button or a key, turned the wheel, pasted or
    /// chose: it now decides the screen and the tabs it watches.
    pub fn operated(&self) {
        let mut state = self.hub.state.lock().expect("live lock poisoned");
        state.order += 1;
        let order = state.order;
        let Some(viewer) = state.viewers.get_mut(&self.id) else {
            return;
        };
        let decided = viewer.operated;
        viewer.operated = order;
        let driving = state.driver == Some(self.id);
        state.driver = Some(self.id);
        // Another viewer's panel may have decided until now.
        let others = state.viewers.len() > 1;
        drop(state);
        if !driving || (decided == 0 && others) {
            self.hub.relayout();
        }
    }

    /// Watches `tab`, or nothing; frames of the watched tab arrive on the
    /// returned queue.
    pub fn watch(&self, tab: Option<&BrowserTab>) -> Option<mpsc::Receiver<Event>> {
        let mut state = self.hub.state.lock().expect("live lock poisoned");
        let previous = state
            .viewers
            .get_mut(&self.id)
            .and_then(|viewer| std::mem::replace(&mut viewer.watching, tab.cloned()));
        if let Some(previous) = previous {
            state.leave_stream(previous.id(), self.id);
        }
        let receiver = tab.map(|tab| {
            let (events, receiver) = stream::events();
            let hub = &self.hub;
            let stream = state.streams.entry(tab.id().clone()).or_insert_with(|| Stream {
                commands: stream::start(tab.clone(), hub.captures.clone(), &hub.tasks),
                viewers: 0,
            });
            stream.viewers += 1;
            let _ended = stream.commands.send(Command::Join {
                viewer: self.id,
                events,
            });
            receiver
        });
        drop(state);
        self.hub.relayout();
        receiver
    }

    pub fn pace(&self, epoch: u32, floor: u32, window: u32) {
        self.hub.state.lock().expect("live lock poisoned").command(
            self.id,
            Command::Pace {
                viewer: self.id,
                epoch,
                floor,
                window,
            },
        );
    }

    pub fn encoding(&self, bitrate: u32, fps: u32, scale: f64) {
        self.hub.state.lock().expect("live lock poisoned").command(
            self.id,
            Command::Encoding {
                viewer: self.id,
                bitrate,
                fps,
                scale,
            },
        );
    }

    pub fn key_frame(&self) {
        self.hub
            .state
            .lock()
            .expect("live lock poisoned")
            .command(self.id, Command::KeyFrame);
    }

    /// Puts `tab` in Web or Mobile mode, sized for whoever decides it.
    pub async fn mode(&self, tab: &BrowserTab, mode: ViewportMode) -> Result<()> {
        let _screen = self.hub.screen.lock().await;
        let panel = {
            let state = self.hub.state.lock().expect("live lock poisoned");
            state
                .decider(tab.id())
                .or_else(|| state.viewers.get(&self.id).and_then(|viewer| viewer.panel))
        };
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
}

impl Drop for Membership {
    fn drop(&mut self) {
        let mut state = self.hub.state.lock().expect("live lock poisoned");
        if let Some(viewer) = state.viewers.remove(&self.id)
            && let Some(tab) = viewer.watching
        {
            state.leave_stream(tab.id(), self.id);
        }
        // The screen keeps its ratio until another viewer operates.
        if state.driver == Some(self.id) {
            state.driver = None;
        }
        drop(state);
        self.hub.relayout();
    }
}
