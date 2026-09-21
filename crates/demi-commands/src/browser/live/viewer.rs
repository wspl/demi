//! One viewer of the conversation's browser (`browser-live-view.md` § The
//! stream): one invocation of `browser.live`, from the page's `hello` until
//! either side ends it.

use std::{collections::VecDeque, sync::Arc, time::Duration};

use chromiumoxide::cdp::browser_protocol::page::EventJavascriptDialogOpening;
use demi_command_service::{
    Input as ServiceInput, InvocationContext, ServiceError,
    protocol::{CommandError, Completion},
};
use tokio::{
    sync::{broadcast, mpsc, watch},
    time::{Instant, MissedTickBehavior},
};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

use super::{
    super::protocol::{
        BrowserViewport, LIVE_STALL_MS, LiveInbound, LiveOutbound, LiveOutboundDialogDialog,
        LiveOutboundStateTabsItem,
    },
    commands::{self, Command},
    frames::{self, Inbound},
    hub::{Membership, Panel},
    input::{Input, Item},
    observers::{self, Observed},
    rate::{Rate, Sample},
    stream::Event,
    uploads,
    writer::Writer,
};
use crate::browser::{
    BrowserEnvironment, BrowserError, BrowserTab, conversations::Controller,
    operation::CONTROL_TIMEOUT, viewport::Mode,
};

/// How long after the viewer's input text the tab copies reaches its
/// clipboard, as a local browser allows a user-activated write.
const COPY_WINDOW: Duration = Duration::from_secs(5);
/// Tab changes arrive in bursts; the viewer hears of them together.
const STATE_DELAY: Duration = Duration::from_millis(100);

enum Failure {
    Cancelled,
    Protocol(String),
}

pub(in crate::browser) async fn serve(
    controller: Arc<Controller>,
    context: InvocationContext,
) -> Result<Completion, ServiceError> {
    let InvocationContext {
        input,
        output,
        cancellation,
        ..
    } = context;
    let (writer, writing) = Writer::start(output);
    let (inbound, uploads, _reading) = read(input);
    let mut viewer = Viewer {
        controller,
        cancel: cancellation,
        writer,
        inbound,
        uploads: Some(uploads),
        panel: None,
        mac: false,
    };
    let result = viewer.run().await;
    // The writer delivers what is queued, such as the reason the view ended.
    drop(viewer);
    let _flushed = tokio::time::timeout(CONTROL_TIMEOUT, writing).await;
    match result {
        Ok(()) => Ok(Completion {
            exit_code: 0,
            error: None,
        }),
        Err(Failure::Cancelled) => Err(ServiceError::Cancelled),
        Err(Failure::Protocol(message)) => Ok(Completion {
            exit_code: 2,
            error: Some(CommandError {
                code: "invalid_input".into(),
                message,
            }),
        }),
    }
}

type Inbounds = mpsc::Receiver<Result<LiveInbound, String>>;

/// Splits the page's bytes into messages; chosen files go to the uploads.
fn read(
    input: ServiceInput,
) -> (
    Inbounds,
    mpsc::Receiver<uploads::Item>,
    AbortOnDropHandle<()>,
) {
    let (messages, inbound) = mpsc::channel(64);
    let (files, uploads) = mpsc::channel(64);
    let task = tokio::spawn(async move {
        let mut reader = frames::Reader::new(input);
        loop {
            let delivered = match reader.next().await {
                Ok(None) => break,
                Err(error) => {
                    let _ended = messages.send(Err(error.to_string())).await;
                    break;
                }
                Ok(Some(Inbound::Control(LiveInbound::Upload {
                    tab,
                    token,
                    revision,
                    upload,
                    files: chosen,
                }))) => files
                    .send(uploads::Item::Start {
                        tab,
                        token,
                        revision,
                        upload,
                        files: chosen,
                    })
                    .await
                    .is_ok(),
                Ok(Some(Inbound::File { upload, file, data })) => files
                    .send(uploads::Item::Data { upload, file, data })
                    .await
                    .is_ok(),
                Ok(Some(Inbound::Control(message))) => messages.send(Ok(message)).await.is_ok(),
            };
            if !delivered {
                break;
            }
        }
    });
    (inbound, uploads, AbortOnDropHandle::new(task))
}

struct Viewer {
    controller: Arc<Controller>,
    cancel: CancellationToken,
    writer: Writer,
    inbound: Inbounds,
    uploads: Option<mpsc::Receiver<uploads::Item>>,
    panel: Option<Panel>,
    mac: bool,
}

impl Viewer {
    /// The page's next message, or none once it ended its side.
    async fn next(&mut self) -> Result<Option<LiveInbound>, Failure> {
        match self.inbound.recv().await {
            Some(Ok(message)) => Ok(Some(message)),
            Some(Err(error)) => Err(Failure::Protocol(error)),
            None => Ok(None),
        }
    }

    async fn run(&mut self) -> Result<(), Failure> {
        match self.next().await? {
            Some(LiveInbound::Hello { platform }) => self.mac = platform == "mac",
            Some(_) => return Err(Failure::Protocol("a view starts with hello".into())),
            None => return Ok(()),
        }
        let Some(environment) = self.environment().await? else {
            return Ok(());
        };
        self.view(environment).await
    }

    async fn end(&self, reason: &str) {
        self.writer
            .control(&LiveOutbound::Ended {
                reason: reason.into(),
            })
            .await;
    }

    /// The conversation's browser; without one, the page hears that none runs,
    /// and the view waits for one: the page starts it by opening a tab with a
    /// request (`web-api.md` § Conversation browser tabs).
    async fn environment(&mut self) -> Result<Option<BrowserEnvironment>, Failure> {
        loop {
            let mut started = self.controller.started();
            match self.controller.environment(None, &self.cancel).await {
                Ok(Some(environment)) => return Ok(Some(environment)),
                Err(BrowserError::Cancelled) => return Err(Failure::Cancelled),
                Err(BrowserError::Closed) => {
                    self.end("released").await;
                    return Ok(None);
                }
                // A browser that failed to start or was lost is not running.
                Ok(None) | Err(_) => {}
            }
            self.writer
                .control(&LiveOutbound::State {
                    running: false,
                    tabs: Vec::new(),
                    watched: None,
                })
                .await;
            let controller = self.controller.clone();
            let cancel = self.cancel.clone();
            loop {
                let message = tokio::select! {
                    _ = controller.cancellation.cancelled() => {
                        self.end("released").await;
                        return Ok(None);
                    }
                    _ = cancel.cancelled() => return Err(Failure::Cancelled),
                    _ = started.changed() => break,
                    message = self.inbound.recv() => match message {
                        Some(Ok(message)) => Some(message),
                        Some(Err(error)) => return Err(Failure::Protocol(error)),
                        None => None,
                    },
                };
                match message {
                    None => return Ok(None),
                    Some(message @ LiveInbound::Panel { .. }) => self.panel = panel(&message),
                    // Nothing else acts without a browser.
                    Some(_) => {}
                }
            }
        }
    }

    async fn view(&mut self, environment: BrowserEnvironment) -> Result<(), Failure> {
        let membership = Arc::new(environment.live.join());
        if let Some(panel) = self.panel {
            membership.panel(panel);
        }
        let mut changes = environment.live.subscribe();
        let (input, input_task) = Input::start(self.writer.clone(), self.mac);
        let commands = commands::start(
            environment.clone(),
            Arc::downgrade(&membership),
            self.writer.clone(),
        );
        let _uploads = self.uploads.take().map(|items| {
            uploads::start(
                environment.clone(),
                membership.clone(),
                self.writer.clone(),
                items,
            )
        });
        let mut session = Session {
            environment: &environment,
            membership: &membership,
            input: &input,
            writer: &self.writer,
            watched: None,
            delivery: Delivery::new(),
            operated: None,
            pasted: None,
        };
        session.state().await;
        let mut refresh: Option<Instant> = None;
        let mut ticks = tokio::time::interval(Duration::from_secs(1));
        ticks.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let result = loop {
            tokio::select! {
                biased;
                _ = self.controller.cancellation.cancelled() => {
                    self.end("released").await;
                    break Ok(());
                }
                _ = environment.ended.cancelled() => {
                    self.end("browser_ended").await;
                    break Ok(());
                }
                _ = self.cancel.cancelled() => break Err(Failure::Cancelled),
                message = self.inbound.recv() => match message {
                    None => break Ok(()),
                    Some(Err(error)) => break Err(Failure::Protocol(error)),
                    Some(Ok(message)) => match message {
                        LiveInbound::Panel { .. } => {
                            if let Some(panel) = panel(&message) {
                                self.panel = Some(panel);
                                membership.panel(panel);
                            }
                        }
                        message => session.receive(message, &commands).await,
                    },
                },
                update = Watched::next(&mut session.watched) => session.update(update).await,
                _ = changes.changed() => {
                    refresh.get_or_insert_with(|| Instant::now() + STATE_DELAY);
                }
                _ = tokio::time::sleep_until(refresh.unwrap_or_else(Instant::now)), if refresh.is_some() => {
                    refresh = None;
                    session.state().await;
                }
                _ = ticks.tick() => session.tick(),
            }
        };
        // The keys and buttons this viewer holds go up; capture of a tab
        // nobody watches stops as the membership ends.
        input.control(Item::Watch(None)).await;
        drop(input);
        let _released = tokio::time::timeout(CONTROL_TIMEOUT, input_task).await;
        result
    }
}

fn panel(message: &LiveInbound) -> Option<Panel> {
    match *message {
        LiveInbound::Panel {
            width,
            height,
            device_pixel_ratio,
            screen_width,
            screen_height,
        } => Some(Panel {
            width: width as u32,
            height: height as u32,
            ratio: device_pixel_ratio,
            screen_width: screen_width as u32,
            screen_height: screen_height as u32,
        }),
        _ => None,
    }
}

/// The tab a viewer watches and what the viewer hears of it.
struct Watched {
    tab: BrowserTab,
    /// Its pictures; none once its stream ended.
    events: Option<mpsc::Receiver<Event>>,
    dialog: watch::Receiver<Option<Arc<EventJavascriptDialogOpening>>>,
    observed: Option<Observing>,
}

struct Observing {
    controls: watch::Receiver<Vec<super::super::protocol::LiveOutboundControlsControlsItem>>,
    cursor: watch::Receiver<(String, bool)>,
    copies: broadcast::Receiver<String>,
    documents: watch::Receiver<u64>,
}

impl Observing {
    fn new(observed: &Observed) -> Self {
        Self {
            controls: observed.controls.subscribe(),
            cursor: observed.cursor.subscribe(),
            copies: observed.copies.subscribe(),
            documents: observed.documents.subscribe(),
        }
    }
}

enum Update {
    Picture(Event),
    StreamEnded,
    Dialog,
    Controls,
    Cursor,
    Copied(String),
    Document,
}

impl Watched {
    async fn next(watched: &mut Option<Self>) -> Update {
        let Some(watched) = watched else {
            return std::future::pending().await;
        };
        let events = async {
            match watched.events.as_mut() {
                Some(events) => events.recv().await,
                None => std::future::pending().await,
            }
        };
        let observed = async {
            let Some(observed) = watched.observed.as_mut() else {
                return std::future::pending().await;
            };
            loop {
                tokio::select! {
                    Ok(()) = observed.controls.changed() => return Update::Controls,
                    Ok(()) = observed.cursor.changed() => return Update::Cursor,
                    Ok(()) = observed.documents.changed() => return Update::Document,
                    copied = observed.copies.recv() => match copied {
                        Ok(text) => return Update::Copied(text),
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => {
                            return std::future::pending().await;
                        }
                    },
                    else => return std::future::pending().await,
                }
            }
        };
        tokio::select! {
            event = events => match event {
                Some(event) => Update::Picture(event),
                None => Update::StreamEnded,
            },
            Ok(()) = watched.dialog.changed() => Update::Dialog,
            update = observed => update,
        }
    }
}

/// The frames a viewer's page has, and the path they take.
struct Delivery {
    /// Counts the picture sequences the page was told about.
    generation: u32,
    /// The capture the current sequence comes from.
    epoch: u32,
    awaiting_key: bool,
    /// The newest capture sequence seen.
    last: u32,
    /// Frames the page has not acknowledged: sequence, when sent, bytes.
    flight: VecDeque<(u32, Instant, usize)>,
    rate: Rate,
    applied: Option<(u32, u32, f64)>,
    started: Instant,
    ticked: Instant,
    last_ack: Instant,
    round_trip: f64,
    decode_queue: u64,
    frames: u32,
    bytes: usize,
    acknowledged: usize,
    dropped: bool,
}

impl Delivery {
    fn new() -> Self {
        let now = Instant::now();
        Self {
            generation: 0,
            epoch: 0,
            awaiting_key: true,
            last: 0,
            flight: VecDeque::new(),
            rate: Rate::new(1280 * 720),
            applied: None,
            started: now,
            ticked: now,
            last_ack: now,
            round_trip: 0.0,
            decode_queue: 0,
            frames: 0,
            bytes: 0,
            acknowledged: 0,
            dropped: false,
        }
    }

    /// Frames up to the floor are shown or will never be: the capture may
    /// run `window` frames past it.
    fn floor(&self) -> u32 {
        self.flight
            .front()
            .map_or(self.last, |(sequence, _, _)| sequence.saturating_sub(1))
    }
}

struct Session<'a> {
    environment: &'a BrowserEnvironment,
    membership: &'a Membership,
    input: &'a Input,
    writer: &'a Writer,
    watched: Option<Watched>,
    delivery: Delivery,
    /// When the viewer last operated, for text the tab copies.
    operated: Option<Instant>,
    /// What the viewer last pasted, which its clipboard already holds.
    pasted: Option<String>,
}

impl Session<'_> {
    /// The browser's tabs and the one this viewer watches.
    async fn state(&mut self) {
        let listed = match self
            .environment
            .listed(&CancellationToken::new(), CONTROL_TIMEOUT)
            .await
        {
            Ok(listed) => listed,
            Err(error) => {
                // A browser that ended takes its tab list with it; the view
                // ends with it and says so.
                if !self.environment.ended.is_cancelled() {
                    eprintln!("live view tabs: {error}");
                    self.writer.notice(error.code(), &error.to_string()).await;
                }
                return;
            }
        };
        if let Some(watched) = &self.watched
            && !listed.iter().any(|(tab, _)| tab.id() == watched.tab.id())
        {
            self.watch(None).await;
        }
        let tabs = listed
            .iter()
            .map(|(tab, info)| {
                let viewport = tab.viewport();
                LiveOutboundStateTabsItem {
                    id: tab.id(),
                    title: info.title.clone(),
                    url: info.url.clone(),
                    created_by: tab.created_by.clone(),
                    viewport: BrowserViewport {
                        width: u64::from(viewport.width),
                        height: u64::from(viewport.height),
                        device_pixel_ratio: viewport.ratio,
                        mode: viewport.mode.name().into(),
                    },
                }
            })
            .collect();
        self.writer
            .control(&LiveOutbound::State {
                running: true,
                tabs,
                watched: self.watched.as_ref().map(|watched| watched.tab.id()),
            })
            .await;
    }

    async fn watch(&mut self, tab: Option<BrowserTab>) {
        if self.watched.as_ref().map(|watched| watched.tab.id()) == tab.as_ref().map(BrowserTab::id)
        {
            return;
        }
        let events = self.membership.watch(tab.as_ref());
        self.input.control(Item::Watch(tab.clone())).await;
        self.delivery.awaiting_key = true;
        self.delivery.flight.clear();
        self.delivery.last = 0;
        self.watched = match tab {
            None => None,
            Some(tab) => {
                let observed = match observers::observe(&tab, &self.environment.observers).await {
                    Ok(observed) => Some(Observing::new(&observed)),
                    Err(error) => {
                        // Without its observer the viewer gets no cursor, native
                        // controls or copied text of this tab.
                        if !tab.ended.is_cancelled() {
                            eprintln!("live view observer of {}: {error}", tab.id());
                            self.writer.notice(error.code(), &error.to_string()).await;
                        }
                        None
                    }
                };
                Some(Watched {
                    dialog: tab.state.dialog.subscribe(),
                    tab,
                    events,
                    observed,
                })
            }
        };
        if self.watched.is_some() {
            for update in [Update::Dialog, Update::Controls, Update::Cursor] {
                self.update(update).await;
            }
        }
    }

    async fn receive(
        &mut self,
        message: LiveInbound,
        commands: &tokio::sync::mpsc::UnboundedSender<Command>,
    ) {
        match message {
            LiveInbound::Watch { tab } => {
                let tab = match tab {
                    Some(id) => match commands::find(self.environment, &id).await {
                        Ok(tab) => Some(tab),
                        Err(error) => {
                            self.writer.notice(error.code(), &error.to_string()).await;
                            None
                        }
                    },
                    None => None,
                };
                self.watch(tab).await;
                self.state().await;
            }
            LiveInbound::Mode { tab, mode } => {
                let mode = if mode == "mobile" {
                    Mode::Mobile
                } else {
                    Mode::Web
                };
                let _ended = commands.send(Command::Mode { tab, mode });
            }
            LiveInbound::Dialog { tab, accept, text } => {
                if let Some(watched) = &self.watched
                    && watched.tab.id() == tab
                {
                    commands::answer(watched.tab.clone(), accept, text, self.writer.clone());
                }
            }
            LiveInbound::Ack {
                generation,
                sequence,
                decode_queue,
            } => self.acknowledged(generation, sequence, decode_queue),
            LiveInbound::Keyframe { generation } => {
                if u64::from(self.delivery.generation) == generation {
                    self.delivery.awaiting_key = true;
                    self.membership.key_frame();
                }
            }
            LiveInbound::Release {} => self.input.control(Item::Release).await,
            message @ (LiveInbound::Pointer { .. }
            | LiveInbound::Wheel { .. }
            | LiveInbound::Key { .. }
            | LiveInbound::Text { .. }
            | LiveInbound::Composition { .. }
            | LiveInbound::Paste { .. }
            | LiveInbound::Choice { .. }) => {
                if let LiveInbound::Paste { text, .. } = &message {
                    self.pasted = Some(text.clone());
                }
                // Moving the pointer is not operating.
                if !matches!(&message, LiveInbound::Pointer { action, .. } if action == "move") {
                    self.operated = Some(Instant::now());
                    self.membership.operated();
                }
                self.input.send(Item::Message(message));
            }
            LiveInbound::Hello { .. } | LiveInbound::Panel { .. } | LiveInbound::Upload { .. } => {}
        }
    }

    async fn update(&mut self, update: Update) {
        let Some(watched) = &mut self.watched else {
            return;
        };
        let tab = watched.tab.id();
        match update {
            Update::Picture(Event::Restart {
                epoch,
                width,
                height,
            }) => {
                let delivery = &mut self.delivery;
                delivery.generation += 1;
                delivery.epoch = epoch;
                delivery.awaiting_key = true;
                delivery.last = 0;
                delivery.flight.clear();
                delivery.rate.resize(u64::from(width) * u64::from(height));
                self.writer
                    .control(&LiveOutbound::Stream {
                        tab,
                        generation: u64::from(delivery.generation),
                        width: u64::from(width),
                        height: u64::from(height),
                    })
                    .await;
            }
            Update::Picture(Event::Frame(frame)) => {
                let delivery = &mut self.delivery;
                if frame.sequence <= delivery.last {
                    return;
                }
                // A frame the viewer never had breaks the pictures after it.
                if delivery.last != 0 && frame.sequence != delivery.last + 1 && !frame.key {
                    delivery.awaiting_key = true;
                    self.membership.key_frame();
                }
                delivery.last = frame.sequence;
                if delivery.awaiting_key && !frame.key {
                    self.membership
                        .pace(delivery.epoch, delivery.floor(), delivery.rate.window);
                    return;
                }
                let bytes = frames::video(&tab, delivery.generation, frame.sequence, &frame);
                let length = bytes.len();
                if self.writer.video(bytes) {
                    delivery.awaiting_key = false;
                    delivery
                        .flight
                        .push_back((frame.sequence, Instant::now(), length));
                    delivery.frames += 1;
                    delivery.bytes += length;
                } else {
                    delivery.dropped = true;
                    delivery.awaiting_key = true;
                    self.membership.key_frame();
                }
                self.membership
                    .pace(delivery.epoch, delivery.floor(), delivery.rate.window);
            }
            Update::Picture(Event::Unavailable(reason)) => {
                self.writer.notice("capture_unavailable", &reason).await;
            }
            Update::Picture(Event::Failed(reason)) => {
                self.writer.notice("capture_failed", &reason).await;
            }
            Update::StreamEnded => watched.events = None,
            Update::Dialog => {
                let dialog = watched.dialog.borrow_and_update().as_ref().map(|dialog| {
                    LiveOutboundDialogDialog {
                        r#type: dialog.r#type.as_ref().to_owned(),
                        message: dialog.message.clone(),
                        default_text: dialog.default_prompt.clone().unwrap_or_default(),
                    }
                });
                self.writer
                    .control(&LiveOutbound::Dialog { tab, dialog })
                    .await;
            }
            Update::Controls => {
                let Some(observed) = &mut watched.observed else {
                    return;
                };
                let controls = observed.controls.borrow_and_update().clone();
                self.writer
                    .control(&LiveOutbound::Controls { tab, controls })
                    .await;
            }
            Update::Cursor => {
                let Some(observed) = &mut watched.observed else {
                    return;
                };
                let (cursor, editable) = observed.cursor.borrow_and_update().clone();
                self.writer
                    .control(&LiveOutbound::Cursor {
                        tab,
                        cursor,
                        editable,
                    })
                    .await;
            }
            Update::Copied(text) => {
                // A paste puts the viewer's own text on the browser's clipboard.
                if self
                    .operated
                    .is_some_and(|operated| operated.elapsed() <= COPY_WINDOW)
                    && self.pasted.as_ref() != Some(&text)
                {
                    self.writer.control(&LiveOutbound::Clipboard { text }).await;
                }
            }
            // Input held in the old document is gone with it.
            Update::Document => self.input.control(Item::Release).await,
        }
    }

    fn acknowledged(&mut self, generation: u64, sequence: u64, decode_queue: u64) {
        let delivery = &mut self.delivery;
        if u64::from(delivery.generation) != generation {
            return;
        }
        let now = Instant::now();
        while let Some(&(sent, at, length)) = delivery.flight.front()
            && u64::from(sent) <= sequence
        {
            delivery.flight.pop_front();
            delivery.acknowledged += length;
            if u64::from(sent) == sequence {
                let round_trip = (now - at).as_secs_f64() * 1000.0;
                delivery.rate.acknowledged(round_trip);
                delivery.round_trip = round_trip;
            }
        }
        delivery.decode_queue = decode_queue;
        delivery.last_ack = now;
        self.membership
            .pace(delivery.epoch, delivery.floor(), delivery.rate.window);
    }

    /// Adapts the picture to the path once a second.
    fn tick(&mut self) {
        let delivery = &mut self.delivery;
        let now = Instant::now();
        let elapsed = now - delivery.ticked;
        delivery.ticked = now;
        let stall = Duration::from_millis(LIVE_STALL_MS);
        let oldest = delivery.flight.front().map(|(_, sent, _)| *sent);
        // A Host pause, or a page that acknowledges nothing, is a stall, not
        // congestion: the budget stays. Frames the page has not acknowledged
        // for long never hold back the capture; the page resumes from a key
        // frame.
        let stalled = elapsed > 3 * Duration::from_secs(1)
            || oldest.is_some_and(|sent| now - sent > stall && delivery.last_ack < sent);
        if stalled {
            if oldest.is_some_and(|sent| now - sent > 2 * stall) {
                delivery.flight.clear();
                delivery.awaiting_key = true;
                self.membership
                    .pace(delivery.epoch, delivery.floor(), delivery.rate.window);
            }
        } else {
            let seconds = elapsed.as_secs_f64().max(0.001);
            delivery.rate.update(&Sample {
                now: (now - delivery.started).as_secs_f64() * 1000.0,
                buffered_bytes: self.writer.queued(),
                ack_age: oldest.map_or(0.0, |sent| (now - sent).as_secs_f64() * 1000.0),
                round_trip: if delivery.frames > 0 {
                    delivery.round_trip
                } else {
                    0.0
                },
                decode_queue: delivery.decode_queue,
                active_frames: delivery.frames,
                congested: delivery.dropped,
                encoded_bitrate: delivery.bytes as f64 * 8.0 / seconds,
                delivered_bitrate: delivery.acknowledged as f64 * 8.0 / seconds,
            });
            let encoding = (
                delivery.rate.bitrate(),
                delivery.rate.fps(),
                delivery.rate.scale(),
            );
            if delivery.applied != Some(encoding) {
                delivery.applied = Some(encoding);
                self.membership.encoding(encoding.0, encoding.1, encoding.2);
            }
        }
        delivery.frames = 0;
        delivery.bytes = 0;
        delivery.acknowledged = 0;
        delivery.dropped = false;
    }
}
