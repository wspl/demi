//! A tab's open JavaScript dialog and the key and button releases it holds
//! back (`browser.md` § JavaScript dialogs, `live-view.md` § Input).
//!
//! One task per tab owns both. The page takes no input while a dialog is
//! open, so a command or a viewer that would release a key or a button
//! then hands the release to the owner, which delivers it once the dialog
//! is answered. Answering is a request to the owner too, so the releases
//! go out in the order they were held back, and the first answer wins,
//! whether the agent's or a viewer's.

use std::{collections::VecDeque, sync::Arc};

use chromiumoxide::{
    Page,
    cdp::browser_protocol::{
        input::{DispatchKeyEventParams, DispatchMouseEventParams},
        page::{EventJavascriptDialogOpening, HandleJavaScriptDialogParams},
    },
    listeners::{EventStream, EventStreamError},
};
use futures_util::StreamExt;
use tokio::sync::{mpsc, oneshot, watch};
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use super::{BrowserError, Result, operation::CONTROL_TIMEOUT};

/// Requests waiting for the owner; a full queue holds back their senders.
const REQUESTS: usize = 16;

/// A key or button release a dialog held back.
#[derive(Clone)]
pub(super) enum InputRelease {
    Mouse(DispatchMouseEventParams),
    Key(DispatchKeyEventParams),
}

type Opening = Option<Arc<EventJavascriptDialogOpening>>;

enum Request {
    Defer(Vec<InputRelease>),
    Answer {
        dialog: Arc<EventJavascriptDialogOpening>,
        accept: bool,
        text: Option<String>,
        reply: oneshot::Sender<Result<()>>,
    },
}

/// The way to a tab's dialog owner, and the dialog it publishes.
pub(super) struct DialogInput {
    requests: mpsc::Sender<Request>,
    dialog: watch::Receiver<Opening>,
}

impl DialogInput {
    /// Starts the owner of `page`'s dialogs; it ends with the tab.
    pub fn start(
        page: Page,
        opening: EventStream<EventJavascriptDialogOpening>,
        ended: CancellationToken,
        tasks: &TaskTracker,
        failure: watch::Sender<Option<String>>,
    ) -> Self {
        let (requests, received) = mpsc::channel(REQUESTS);
        let (published, dialog) = watch::channel(None);
        let owner = Owner {
            page,
            opening,
            published,
            deferred: VecDeque::new(),
            ended,
            failure,
        };
        tasks.spawn(owner.run(received));
        Self { requests, dialog }
    }

    /// The open dialog, if one is.
    pub fn open(&self) -> Opening {
        self.dialog.borrow().clone()
    }

    pub fn is_open(&self) -> bool {
        self.dialog.borrow().is_some()
    }

    /// Watches the open dialog from now on.
    pub fn watch(&self) -> watch::Receiver<Opening> {
        let mut dialog = self.dialog.clone();
        dialog.mark_unchanged();
        dialog
    }

    /// Delivers `releases` once no dialog is open, after those held back
    /// before them.
    pub async fn defer(&self, releases: Vec<InputRelease>) {
        // The owner ends with its tab, whose input no longer matters then.
        let _ended = self.requests.send(Request::Defer(releases)).await;
    }

    /// Answers `dialog` unless someone answered it first, then delivers the
    /// releases it held back.
    pub async fn answer(
        &self,
        dialog: &Arc<EventJavascriptDialogOpening>,
        accept: bool,
        text: Option<String>,
    ) -> Result<()> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Request::Answer {
                dialog: dialog.clone(),
                accept,
                text,
                reply,
            })
            .await
            .map_err(|_| BrowserError::Closed)?;
        answer.await.map_err(|_| BrowserError::Closed)?
    }
}

struct Owner {
    page: Page,
    /// Headless dialogs are closed only by this owner. Answering clears the
    /// exact dialog it answered, so a later prompt is not erased by an
    /// earlier dialog's close.
    opening: EventStream<EventJavascriptDialogOpening>,
    published: watch::Sender<Opening>,
    deferred: VecDeque<InputRelease>,
    ended: CancellationToken,
    failure: watch::Sender<Option<String>>,
}

impl Owner {
    async fn run(mut self, mut requests: mpsc::Receiver<Request>) {
        loop {
            tokio::select! {
                biased;
                _ = self.ended.cancelled() => break,
                event = self.opening.next() => {
                    if self.opened(event).is_err() {
                        break;
                    }
                }
                request = requests.recv() => match request {
                    Some(Request::Defer(releases)) => {
                        self.deferred.extend(releases);
                        if let Err(error) = self.deliver().await {
                            tracing::warn!("a dialog's held input was not delivered: {error}");
                        }
                    }
                    Some(Request::Answer {
                        dialog,
                        accept,
                        text,
                        reply,
                    }) => {
                        let answered = self.answer(&dialog, accept, text).await;
                        let answered = match answered {
                            Ok(()) => self.deliver().await,
                            Err(error) => Err(error),
                        };
                        // An answerer that left needs no reply.
                        let _left = reply.send(answered);
                    }
                    None => break,
                },
            }
        }
    }

    /// Publishes a dialog that opened. A lost observation fails the tab:
    /// its dialogs can no longer be known.
    fn opened(
        &mut self,
        event: Option<std::result::Result<Arc<EventJavascriptDialogOpening>, EventStreamError>>,
    ) -> Result<()> {
        match event {
            Some(Ok(dialog)) => {
                self.published.send_replace(Some(dialog));
                Ok(())
            }
            Some(Err(error)) => {
                self.failure
                    .send_replace(Some(format!("browser dialog observation failed: {error}")));
                self.ended.cancel();
                Err(BrowserError::Closed)
            }
            None => Err(BrowserError::Closed),
        }
    }

    async fn answer(
        &mut self,
        dialog: &Arc<EventJavascriptDialogOpening>,
        accept: bool,
        text: Option<String>,
    ) -> Result<()> {
        // The first answer wins: a dialog someone answered is gone.
        if !self
            .published
            .borrow()
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, dialog))
        {
            return Err(BrowserError::DialogNotFound);
        }
        let mut request = HandleJavaScriptDialogParams::new(accept);
        request.prompt_text = text;
        self.page.execute(request).await?;
        self.published.send_if_modified(|current| {
            if current
                .as_ref()
                .is_some_and(|current| Arc::ptr_eq(current, dialog))
            {
                *current = None;
                true
            } else {
                false
            }
        });
        Ok(())
    }

    /// Delivers the held-back releases in order while no dialog is open. A
    /// release that opens another dialog was delivered; the rest wait for
    /// that dialog.
    async fn deliver(&mut self) -> Result<()> {
        while self.published.borrow().is_none()
            && let Some(release) = self.deferred.front().cloned()
        {
            let page = self.page.clone();
            let sent = async move {
                match release {
                    InputRelease::Mouse(event) => page.execute(event).await.map(|_| ()),
                    InputRelease::Key(event) => page.execute(event).await.map(|_| ()),
                }
            };
            // Chrome does not answer an input event whose handler opened a
            // dialog until that dialog closes.
            tokio::select! {
                biased;
                event = self.opening.next() => {
                    self.deferred.pop_front();
                    self.opened(event)?;
                }
                sent = tokio::time::timeout(CONTROL_TIMEOUT, sent) => {
                    self.deferred.pop_front();
                    sent.map_err(|_| BrowserError::Timeout)??;
                }
            }
        }
        Ok(())
    }
}
