//! One viewer of the conversation's browser: one invocation of `browser.live`,
//! from the page's `hello` until either side ends it.
//!
//! The viewer's input and the agent's are not exclusive: both reach the page in
//! the order they arrive and the operation lock orders only agent commands, so
//! interleaving is expected (a `fill` may type into the field the user clicked
//! meanwhile). Held keys and buttons are tracked per source, and a source's end
//! releases only what it holds. For a `user` caller `open`, `goto`, `back`,
//! `forward` and `reload` answer without waiting for the page to load: a user
//! waits in the panel, not in a script.

use std::{future::Future, sync::Arc, time::Duration};

use chromiumoxide::cdp::browser_protocol::page::EventJavascriptDialogOpening;
use tokio::{
    sync::{broadcast, mpsc, oneshot, watch},
    time::Instant,
};
use tokio_util::{sync::CancellationToken, task::AbortOnDropHandle};

use crate::driver::operation::{BrowserError, CONTROL_TIMEOUT, Result as BrowserResult};
use crate::tabs::{environment::BrowserEnvironment, tab::BrowserTab};
use demi_command_package_browser_protocol::live::{
    CursorRegion, EndReason, LiveControl, LiveDialog, LiveDownload, LiveMenu, LiveModuleMessage,
    LiveTab, LiveViewerMessage, ViewerMode,
};
use demi_live_view::{delivery::Delivery, input::Item};
use demi_live_view_protocol::{
    ChunkAck, LiveHello, LiveKeyframe, LiveRelease, Platform, PointerAction, SourceId, ViewerInput,
};
use demi_command_protocol::{CommandError, Completion};
use demi_command_sdk::{Input as ServiceInput, InvocationContext, ServiceError};

use crate::live::protocol::{TabId, ViewportMode};
use crate::live::{
    commands::{self, Command},
    frames::{self, Inbound},
    hub::{Hub, Membership, Panel},
    input::{BrowserInput, Input},
    observers::Observed,
    stream::{Event, StreamView},
    uploads,
    writer::Writer,
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

/// The conversation's browser as its viewers reach it, which the program
/// implements: the live view does not know how conversations are owned or
/// released.
pub trait ViewedBrowser: Send + Sync + 'static {
    /// The running browser and its live view hub; none while no browser
    /// runs. `Closed` once the conversation is released.
    fn running(
        &self,
        cancel: &CancellationToken,
    ) -> impl Future<Output = BrowserResult<Option<(BrowserEnvironment, Arc<Hub>)>>> + Send;

    /// Resolves once a browser starts or ends after this call.
    fn changed(&self) -> impl Future<Output = ()> + Send + 'static;

    /// Cancelled when the conversation's release arrives.
    fn released(&self) -> &CancellationToken;

    /// The number of the conversation's latest tab list, which a list sent
    /// while no browser runs carries: the numbers only grow within the
    /// conversation.
    fn list_number(&self) -> u64;
}

/// Serves one view of `browser`, the invocation of `browser.live` that
/// `context` carries, until the page or the conversation ends it.
pub async fn serve<B: ViewedBrowser>(
    browser: Arc<B>,
    context: InvocationContext,
) -> Result<Completion, ServiceError> {
    let InvocationContext {
        input,
        output,
        cancellation,
        ..
    } = context;
    let (writer, writing) = Writer::start(output);
    let (inbound, acks, uploads, _reading) = read(input);
    let mut viewer = Viewer {
        browser,
        cancel: cancellation,
        writer,
        inbound,
        acks,
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

type Inbounds = mpsc::Receiver<Result<LiveViewerMessage, String>>;

/// Splits the page's bytes into messages; acknowledgements go to the
/// delivery, chosen files to the uploads.
fn read(
    input: ServiceInput,
) -> (
    Inbounds,
    mpsc::Receiver<Vec<ChunkAck>>,
    mpsc::Receiver<uploads::Item>,
    AbortOnDropHandle<()>,
) {
    let (messages, inbound) = mpsc::channel(64);
    let (acknowledged, acks) = mpsc::channel(64);
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
                Ok(Some(Inbound::Control(LiveViewerMessage::Upload {
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
                Ok(Some(Inbound::Acks(batch))) => acknowledged.send(batch).await.is_ok(),
            };
            if !delivered {
                break;
            }
        }
    });
    (inbound, acks, uploads, AbortOnDropHandle::new(task))
}

struct Viewer<B> {
    browser: Arc<B>,
    cancel: CancellationToken,
    writer: Writer,
    inbound: Inbounds,
    acks: mpsc::Receiver<Vec<ChunkAck>>,
    uploads: Option<mpsc::Receiver<uploads::Item>>,
    panel: Option<Panel>,
    mac: bool,
}

impl<B: ViewedBrowser> Viewer<B> {
    /// The page's next message, or none once it ended its side.
    async fn next(&mut self) -> Result<Option<LiveViewerMessage>, Failure> {
        match self.inbound.recv().await {
            Some(Ok(message)) => Ok(Some(message)),
            Some(Err(error)) => Err(Failure::Protocol(error)),
            None => Ok(None),
        }
    }

    async fn run(&mut self) -> Result<(), Failure> {
        match self.next().await? {
            Some(LiveViewerMessage::Hello(LiveHello { platform })) => {
                self.mac = platform == Platform::Mac;
            }
            Some(_) => return Err(Failure::Protocol("a view starts with hello".into())),
            None => return Ok(()),
        }
        let Some((environment, live)) = self.environment().await? else {
            return Ok(());
        };
        self.view(environment, &live).await
    }

    async fn end(&self, reason: EndReason) {
        self.writer
            .control(&LiveModuleMessage::Ended { reason })
            .await;
    }

    /// The conversation's browser; without one, the page hears that none runs,
    /// and the view waits for one: the page starts it by opening a tab with a
    /// request.
    async fn environment(&mut self) -> Result<Option<(BrowserEnvironment, Arc<Hub>)>, Failure> {
        loop {
            let changed = self.browser.changed();
            tokio::pin!(changed);
            match self.browser.running(&self.cancel).await {
                Ok(Some(running)) => return Ok(Some(running)),
                Err(BrowserError::Cancelled) => return Err(Failure::Cancelled),
                Err(BrowserError::Closed) => {
                    self.end(EndReason::Released).await;
                    return Ok(None);
                }
                // A browser that failed to start or was lost is not running.
                Ok(None) | Err(_) => {}
            }
            self.writer
                .control(&LiveModuleMessage::State {
                    running: false,
                    list: self.browser.list_number(),
                    tabs: Vec::new(),
                    watched: None,
                })
                .await;
            let browser = self.browser.clone();
            let cancel = self.cancel.clone();
            loop {
                let message = tokio::select! {
                    _ = browser.released().cancelled() => {
                        self.end(EndReason::Released).await;
                        return Ok(None);
                    }
                    _ = cancel.cancelled() => return Err(Failure::Cancelled),
                    // A browser started or ended.
                    () = &mut changed => break,
                    message = self.inbound.recv() => match message {
                        Some(Ok(message)) => Some(message),
                        Some(Err(error)) => return Err(Failure::Protocol(error)),
                        None => None,
                    },
                };
                match message {
                    None => return Ok(None),
                    Some(message @ LiveViewerMessage::Panel { .. }) => self.panel = panel(&message),
                    // Nothing else acts without a browser.
                    Some(_) => {}
                }
            }
        }
    }

    async fn view(
        &mut self,
        environment: BrowserEnvironment,
        live: &Arc<Hub>,
    ) -> Result<(), Failure> {
        let Ok((membership, mut notices)) = live.join().await else {
            // The hub ends with the browser.
            self.end(EndReason::BrowserEnded).await;
            return Ok(());
        };
        let membership = Arc::new(membership);
        if let Some(panel) = self.panel {
            membership.panel(panel).await;
        }
        let mut changes = live.subscribe();
        let (input, input_task) = Input::start(
            environment.tasks(),
            environment.ended().clone(),
            BrowserInput::new(self.writer.until(environment.ended()), self.mac),
        );
        let commands = commands::start(
            &environment,
            Arc::downgrade(&membership),
            self.writer.clone(),
        );
        // The view's uploads end with it, or with the environment.
        let stop_uploads = environment.ended().child_token();
        let _stop_uploads = stop_uploads.clone().drop_guard();
        if let Some(items) = self.uploads.take() {
            uploads::start(
                environment.clone(),
                membership.clone(),
                self.writer.clone(),
                items,
                stop_uploads,
            );
        }
        let mut session = Session {
            environment: &environment,
            live,
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
        let result = loop {
            let due = session.delivery.due();
            tokio::select! {
                biased;
                _ = self.browser.released().cancelled() => {
                    self.end(EndReason::Released).await;
                    break Ok(());
                }
                _ = environment.ended().cancelled() => {
                    self.end(EndReason::BrowserEnded).await;
                    break Ok(());
                }
                _ = self.cancel.cancelled() => break Err(Failure::Cancelled),
                message = self.inbound.recv() => match message {
                    None => break Ok(()),
                    Some(Err(error)) => break Err(Failure::Protocol(error)),
                    Some(Ok(message)) => match message {
                        LiveViewerMessage::Panel { .. } => {
                            if let Some(panel) = panel(&message) {
                                self.panel = Some(panel);
                                membership.panel(panel).await;
                            }
                        }
                        message => session.receive(message, &commands).await,
                    },
                },
                update = Watched::next(&mut session.watched) => session.update(update).await,
                // What the hub could not do for this viewer.
                Some(notice) = notices.recv() => {
                    self.writer.notice(notice.code, &notice.message).await;
                }
                _ = changes.changed() => {
                    refresh.get_or_insert_with(|| Instant::now() + STATE_DELAY);
                }
                _ = tokio::time::sleep_until(refresh.unwrap_or_else(Instant::now)), if refresh.is_some() => {
                    refresh = None;
                    session.state().await;
                }
                Some(acks) = self.acks.recv() => {
                    session.delivery.acknowledged(&acks, stream(&session.watched));
                }
                _ = tokio::time::sleep_until(due.unwrap_or_else(Instant::now)), if due.is_some() => {
                    session.delivery.release(session.writer);
                }
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

fn panel(message: &LiveViewerMessage) -> Option<Panel> {
    match *message {
        LiveViewerMessage::Panel {
            width,
            height,
            device_pixel_ratio,
            screen_width,
            screen_height,
        } => Some(Panel {
            width,
            height,
            ratio: device_pixel_ratio,
            screen_width,
            screen_height,
        }),
        _ => None,
    }
}

/// The tab a viewer watches and what the viewer hears of it.
struct Watched {
    tab: BrowserTab,
    /// The tab as its pictures name it.
    source: SourceId,
    /// Its pictures and the pacing they follow; none once its stream ended.
    stream: Option<StreamView>,
    dialog: watch::Receiver<Option<Arc<EventJavascriptDialogOpening>>>,
    observed: Option<Observing>,
    /// The tab's observer while it starts: Chrome holds the commands that
    /// start it until a navigation under way commits, and the view never
    /// waits for them.
    starting: Option<oneshot::Receiver<BrowserResult<Arc<Observed>>>>,
}

struct Observing {
    observed: Arc<Observed>,
    controls: watch::Receiver<Vec<LiveControl>>,
    cursor: watch::Receiver<(String, bool)>,
    regions: watch::Receiver<Vec<CursorRegion>>,
    copies: broadcast::Receiver<String>,
    menus: broadcast::Receiver<LiveMenu>,
    downloads: watch::Receiver<Vec<LiveDownload>>,
    documents: watch::Receiver<u64>,
}

impl Observing {
    fn new(observed: Arc<Observed>) -> Self {
        Self {
            controls: observed.controls.subscribe(),
            cursor: observed.cursor.subscribe(),
            regions: observed.regions.subscribe(),
            copies: observed.copies.subscribe(),
            menus: observed.menus.subscribe(),
            downloads: observed.downloads.subscribe(),
            documents: observed.documents.subscribe(),
            observed,
        }
    }
}

enum Update {
    Picture(Event),
    /// The tab's observer started, or could not.
    Observed(BrowserResult<Arc<Observed>>),
    StreamEnded,
    Dialog,
    Controls,
    Cursor,
    Regions,
    Copied(String),
    Menu(LiveMenu),
    Downloads,
    Document,
}

impl Watched {
    async fn next(watched: &mut Option<Self>) -> Update {
        let Some(watched) = watched else {
            return std::future::pending().await;
        };
        let events = async {
            match watched.stream.as_mut() {
                Some(stream) => stream.events.recv().await,
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
                    Ok(()) = observed.regions.changed() => return Update::Regions,
                    Ok(()) = observed.documents.changed() => return Update::Document,
                    Ok(()) = observed.downloads.changed() => return Update::Downloads,
                    menu = observed.menus.recv() => match menu {
                        Ok(menu) => return Update::Menu(menu),
                        Err(broadcast::error::RecvError::Lagged(_)) => {}
                        Err(broadcast::error::RecvError::Closed) => {
                            return std::future::pending().await;
                        }
                    },
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
        let starting = async {
            match watched.starting.as_mut() {
                Some(starting) => starting.await,
                None => std::future::pending().await,
            }
        };
        tokio::select! {
            event = events => match event {
                Some(event) => Update::Picture(event),
                None => Update::StreamEnded,
            },
            Ok(()) = watched.dialog.changed() => Update::Dialog,
            update = observed => update,
            // A start the tab's end gave up answers nothing.
            Ok(started) = starting => Update::Observed(started),
        }
    }
}

/// Starts the observer of `tab` beside the view, which hears of it once it
/// started. The start ends with the tab: a navigation that never commits
/// holds it until the tab closes or the browser ends.
fn start_observer(
    live: &Arc<Hub>,
    tab: &BrowserTab,
) -> oneshot::Receiver<BrowserResult<Arc<Observed>>> {
    let (started, starting) = oneshot::channel();
    let tasks = live.tasks().clone();
    let live = live.clone();
    let tab = tab.clone();
    tasks.spawn(async move {
        tokio::select! {
            () = tab.ended().cancelled() => {}
            observed = live.observer(&tab) => {
                // A viewer that moved on no longer waits for it.
                let _left = started.send(observed);
            }
        }
    });
    starting
}

/// The watched tab's stream, while its pictures come.
fn stream(watched: &Option<Watched>) -> Option<&StreamView> {
    watched.as_ref().and_then(|watched| watched.stream.as_ref())
}

struct Session<'a> {
    environment: &'a BrowserEnvironment,
    live: &'a Arc<Hub>,
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
        let (list, listing) = match self
            .environment
            .numbered(&CancellationToken::new(), CONTROL_TIMEOUT)
            .await
        {
            Ok(numbered) => numbered,
            Err(error) => {
                // A browser that ended takes its tab list with it; the view
                // ends with it and says so.
                if !self.environment.ended().is_cancelled() {
                    tracing::warn!("live view tabs: {error}");
                    self.writer.notice(error.code(), &error.to_string()).await;
                }
                return;
            }
        };
        if let Some(watched) = &self.watched
            && listing.find(watched.tab.id()).is_none()
        {
            self.watch(None).await;
        }
        let tabs = listing
            .tabs
            .iter()
            .map(|listed| {
                let (title, url) = listed.shown();
                LiveTab {
                    id: listed.tab.id().clone(),
                    title,
                    url,
                    created_by: listed.tab.created_by().clone(),
                    viewport: listed.tab.viewport(),
                    loading: listed.tab.loading(),
                    can_go_back: listed.tab.history().back,
                    can_go_forward: listed.tab.history().forward,
                    favicon: listed.tab.favicon(),
                }
            })
            .collect();
        self.writer
            .control(&LiveModuleMessage::State {
                running: true,
                list,
                tabs,
                watched: self
                    .watched
                    .as_ref()
                    .map(|watched| watched.tab.id().clone()),
            })
            .await;
    }

    async fn watch(&mut self, tab: Option<BrowserTab>) {
        if self.watched.as_ref().map(|watched| watched.tab.id()) == tab.as_ref().map(BrowserTab::id)
        {
            return;
        }
        let stream = self.membership.watch(tab.as_ref()).await;
        // The encoding follows the viewer's path, whichever tab it watches.
        if let (Some(stream), Some((bitrate, fps, scale))) = (&stream, self.delivery.applied()) {
            stream.encoding(bitrate, fps, scale);
        }
        self.input.control(Item::Watch(tab.clone())).await;
        self.delivery.rewatch();
        self.watched = tab.map(|tab| Watched {
            source: SourceId::try_from(tab.id().as_str())
                .expect("a tab's ID fits a picture's source"),
            dialog: tab.state().dialog.watch(),
            starting: Some(start_observer(self.live, &tab)),
            tab,
            stream,
            observed: None,
        });
        if self.watched.is_some() {
            self.update(Update::Dialog).await;
        }
    }

    async fn receive(&mut self, message: LiveViewerMessage, commands: &mpsc::Sender<Command>) {
        match message {
            LiveViewerMessage::Watch { tab } => {
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
            LiveViewerMessage::Mode { tab, mode } => {
                let mode = match mode {
                    ViewerMode::Mobile => ViewportMode::Mobile,
                    ViewerMode::Web => ViewportMode::Web,
                };
                // The runner ends with the browser, as the view does.
                let _ended = commands.send(Command::Mode { tab, mode }).await;
            }
            LiveViewerMessage::Dialog { tab, accept, text } => {
                if let Some(watched) = &self.watched
                    && watched.tab.id() == &tab
                {
                    commands::answer(watched.tab.clone(), accept, text, self.writer.clone());
                }
            }
            LiveViewerMessage::Keyframe(LiveKeyframe { generation }) => {
                self.delivery.key_frame(generation, stream(&self.watched));
            }
            LiveViewerMessage::Release(LiveRelease {}) => self.input.control(Item::Release).await,
            LiveViewerMessage::Recapture {} => self.environment.captures().retry().await,
            message @ (LiveViewerMessage::Input { .. }
            | LiveViewerMessage::Paste { .. }
            | LiveViewerMessage::Choice { .. }) => {
                if let LiveViewerMessage::Paste { text, .. } = &message {
                    self.pasted = Some(text.clone());
                }
                // A new document resolves its cursor where the pointer is.
                if let LiveViewerMessage::Input {
                    tab,
                    input: ViewerInput::Pointer { x, y, .. },
                } = &message
                    && let Some(Watched {
                        tab: watched,
                        observed: Some(observing),
                        ..
                    }) = &self.watched
                    && watched.id() == tab
                {
                    observing.observed.pointer.send_replace(Some((*x, *y)));
                }
                // Moving the pointer is not operating.
                if !matches!(
                    &message,
                    LiveViewerMessage::Input {
                        input: ViewerInput::Pointer {
                            action: PointerAction::Move,
                            ..
                        },
                        ..
                    }
                ) {
                    self.operated = Some(Instant::now());
                    self.membership.operated();
                }
                self.input.send(message);
            }
            LiveViewerMessage::Hello(_)
            | LiveViewerMessage::Panel { .. }
            | LiveViewerMessage::Upload { .. } => {}
        }
    }

    async fn update(&mut self, update: Update) {
        if let Update::Observed(started) = update {
            self.observed(started).await;
            return;
        }
        let Some(watched) = &mut self.watched else {
            return;
        };
        let tab: TabId = watched.tab.id().clone();
        match update {
            Update::Picture(Event::Restart {
                epoch,
                width,
                height,
                layout: viewport,
                scale,
                sequence,
            }) => {
                let generation = self.delivery.restart(
                    epoch,
                    width,
                    height,
                    scale,
                    sequence,
                    watched.stream.as_ref(),
                );
                self.writer
                    .control(&LiveModuleMessage::Stream {
                        tab,
                        generation,
                        width,
                        height,
                        viewport,
                        scale,
                    })
                    .await;
            }
            Update::Picture(Event::Frame(frame)) => {
                self.delivery
                    .frame(&watched.source, &frame, watched.stream.as_ref());
            }
            Update::Picture(Event::Unavailable(reason)) => {
                self.writer
                    .notice(
                        demi_command_package_browser_protocol::live::CAPTURE_UNAVAILABLE,
                        &reason,
                    )
                    .await;
            }
            Update::Picture(Event::Failed(reason)) => {
                self.writer
                    .notice(
                        demi_command_package_browser_protocol::live::CAPTURE_FAILED,
                        &reason,
                    )
                    .await;
            }
            Update::Picture(Event::Stopped(reason)) => {
                self.writer
                    .notice(
                        demi_command_package_browser_protocol::live::CAPTURE_STOPPED,
                        &reason,
                    )
                    .await;
            }
            Update::StreamEnded => watched.stream = None,
            // Taken above, without the borrow of the watched tab.
            Update::Observed(_) => {}
            Update::Dialog => {
                let dialog = watched
                    .dialog
                    .borrow_and_update()
                    .as_ref()
                    .map(|dialog| LiveDialog {
                        r#type: crate::tabs::tab::dialog_type(&dialog.r#type),
                        message: dialog.message.clone(),
                        default_text: dialog.default_prompt.clone().unwrap_or_default(),
                    });
                self.writer
                    .control(&LiveModuleMessage::Dialog { tab, dialog })
                    .await;
            }
            Update::Controls => {
                let Some(observed) = &mut watched.observed else {
                    return;
                };
                let controls = observed.controls.borrow_and_update().clone();
                self.writer
                    .control(&LiveModuleMessage::Controls { tab, controls })
                    .await;
            }
            Update::Cursor => {
                let Some(observed) = &mut watched.observed else {
                    return;
                };
                let (cursor, editable) = observed.cursor.borrow_and_update().clone();
                self.writer
                    .control(&LiveModuleMessage::Cursor {
                        tab,
                        cursor,
                        editable,
                    })
                    .await;
            }
            Update::Regions => {
                let Some(observed) = &mut watched.observed else {
                    return;
                };
                let regions = observed.regions.borrow_and_update().clone();
                self.writer
                    .control(&LiveModuleMessage::Cursors { tab, regions })
                    .await;
            }
            Update::Copied(text) => {
                // A paste puts the viewer's own text on the browser's clipboard.
                if self
                    .operated
                    .is_some_and(|operated| operated.elapsed() <= COPY_WINDOW)
                    && self.pasted.as_ref() != Some(&text)
                {
                    self.writer
                        .control(&LiveModuleMessage::Clipboard { text })
                        .await;
                }
            }
            // A right click is this viewer's when it operated just now, as
            // the copies it hears are; the agent's own right clicks open no
            // menu on the viewer's page.
            Update::Menu(menu) => {
                if self
                    .operated
                    .is_some_and(|operated| operated.elapsed() <= COPY_WINDOW)
                {
                    self.writer
                        .control(&LiveModuleMessage::Menu { tab, menu })
                        .await;
                }
            }
            Update::Downloads => {
                let Some(observed) = &mut watched.observed else {
                    return;
                };
                let downloads = observed.downloads.borrow_and_update().clone();
                self.writer
                    .control(&LiveModuleMessage::Downloads { tab, downloads })
                    .await;
            }
            // Input held in the old document is gone with it.
            Update::Document => self.input.control(Item::Release).await,
        }
    }

    /// The watched tab's observer started: the viewer hears what it found.
    async fn observed(&mut self, started: BrowserResult<Arc<Observed>>) {
        let Some(watched) = &mut self.watched else {
            return;
        };
        watched.starting = None;
        match started {
            Ok(observed) => watched.observed = Some(Observing::new(observed)),
            // Without its observer the viewer gets no cursor, native controls
            // or copied text of this tab.
            Err(error) => {
                if !watched.tab.ended().is_cancelled() {
                    tracing::warn!("live view observer of {}: {error}", watched.tab.id());
                    self.writer.notice(error.code(), &error.to_string()).await;
                }
                return;
            }
        }
        for update in [
            Update::Controls,
            Update::Regions,
            Update::Cursor,
            Update::Downloads,
        ] {
            Box::pin(self.update(update)).await;
        }
    }

}
