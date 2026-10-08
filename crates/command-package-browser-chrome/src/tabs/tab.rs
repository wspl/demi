//! A browser tab and what it keeps beside its page (`browser.md` § Owners
//! inside the service): its operation lock with the session its holder uses,
//! the way to its debugging owner, its console buffer, its open dialog and
//! its viewport. What acts on the tab's page is written in the modules above
//! `tabs`, as functions over a [`BrowserTab`].

use std::{sync::Arc, time::Duration};

use chromiumoxide::{Page, cdp::browser_protocol::page::EventJavascriptDialogOpening};
use tokio::sync::watch;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::driver::operation::{BrowserError, Operation, Result};

use crate::tabs::{
    dialog::DialogInput,
    environment::BrowserHandle,
    protocol::{BrowserCreatedBy, TabId},
    registry::{Closed, Tabs},
    session::TabGate,
};

/// A JavaScript dialog's type as results and the live view name it.
pub fn dialog_type(
    kind: &chromiumoxide::cdp::browser_protocol::page::DialogType,
) -> crate::tabs::protocol::DialogType {
    use chromiumoxide::cdp::browser_protocol::page::DialogType as Cdp;
    match kind {
        Cdp::Alert => crate::tabs::protocol::DialogType::Alert,
        Cdp::Confirm => crate::tabs::protocol::DialogType::Confirm,
        Cdp::Prompt => crate::tabs::protocol::DialogType::Prompt,
        Cdp::Beforeunload => crate::tabs::protocol::DialogType::BeforeUnload,
    }
}

/// What a tab keeps beside its page; each part has its owner.
pub struct TabState {
    /// Why the browser connection ended, once it ended on its own.
    pub failure: watch::Sender<Option<String>>,
    /// The operation lock and the session its holder uses.
    pub gate: TabGate,
    /// The debugging connections of `cdp` commands and their events.
    pub debug: crate::tabs::debug::DebugSessions,
    pub console: crate::tabs::logs::Console,
    /// The open dialog and the input it holds back.
    pub dialog: DialogInput,
    pub viewport: watch::Sender<crate::tabs::viewport::Viewports>,
    /// Keeps the page's captures and its resizes apart.
    pub capture: crate::driver::capture::CaptureGate,
    /// Where the tab's top-level page is in its loading.
    pub load: watch::Sender<crate::tabs::loading::PageLoad>,
    /// The address a navigation the user started loads, until its document
    /// commits or it ends: what the tab's address bar shows meanwhile.
    pub requested: watch::Sender<Option<String>>,
    /// Which ends of its history the tab is away from.
    pub history: watch::Sender<crate::tabs::history::HistoryEnds>,
    /// The page's icon as a PNG `data:` URL, while it has one.
    pub favicon: watch::Sender<Option<String>>,
    /// How many times the agent showed the tab to the user
    /// (`live-view.md` § Showing a tab).
    pub shows: watch::Sender<u64>,
    /// Counts the changes to what the browser shows, the environment's.
    pub changes: watch::Sender<u64>,
    /// The environment's tasks, which retirement joins: work for the tab that
    /// no command waits for runs on them and ends with the tab.
    pub tasks: TaskTracker,
}

impl TabState {
    pub(crate) async fn observe(
        page: &Page,
        ended: CancellationToken,
        tasks: &TaskTracker,
        failure: watch::Sender<Option<String>>,
        changes: watch::Sender<u64>,
    ) -> Result<Arc<Self>> {
        let opening = page
            .event_listener::<EventJavascriptDialogOpening>()
            .await?;
        let console = crate::tabs::logs::observe(page, ended.clone(), tasks).await?;
        let requested = watch::channel(None).0;
        let load = crate::tabs::loading::observe(
            page,
            ended.clone(),
            tasks,
            changes.clone(),
            requested.clone(),
        )
        .await?;
        let history =
            crate::tabs::history::observe(page, ended.clone(), tasks, changes.clone()).await?;
        let favicon =
            crate::tabs::favicon::observe(page, ended.clone(), tasks, changes.clone()).await?;
        let dialog = DialogInput::start(page.clone(), opening, ended, tasks, failure.clone());
        Ok(Arc::new(Self {
            failure,
            gate: TabGate::default(),
            debug: Default::default(),
            console,
            dialog,
            viewport: watch::channel(Default::default()).0,
            capture: Default::default(),
            load,
            requested,
            history,
            favicon,
            shows: watch::channel(0).0,
            changes,
            tasks: tasks.clone(),
        }))
    }
}

/// A tab of the environment's registry; cloning shares it.
#[derive(Clone)]
pub struct BrowserTab {
    pub(crate) page: Page,
    pub(crate) browser: BrowserHandle,
    /// The registry that lists the tab and closes it.
    registry: Tabs,
    pub(crate) ended: CancellationToken,
    pub(crate) state: Arc<TabState>,
    id: TabId,
    pub(crate) created_by: BrowserCreatedBy,
}

impl BrowserTab {
    pub(crate) fn new(
        page: Page,
        browser: BrowserHandle,
        registry: Tabs,
        ended: CancellationToken,
        state: Arc<TabState>,
        id: TabId,
        created_by: BrowserCreatedBy,
    ) -> Self {
        Self {
            page,
            browser,
            registry,
            ended,
            state,
            id,
            created_by,
        }
    }

    /// The pages this tab's page opened, registered or not.
    pub async fn opened(&self) -> Result<Vec<TabId>> {
        self.registry.opened_by(self.page.target_id().clone()).await
    }

    /// The tabs this tab's page opened, once each is registered.
    pub async fn popups(&self) -> Result<Vec<TabId>> {
        self.registry.popups(self.page.target_id().clone()).await
    }

    /// An operation in this tab's lifetime until `deadline`, which reports
    /// the browser transport's cause when the tab ends mid-command.
    pub fn operation<'a>(
        &'a self,
        cancelled: &'a CancellationToken,
        deadline: tokio::time::Instant,
    ) -> Operation<'a> {
        Operation::until(&self.ended, cancelled, deadline).reporting(&self.state.failure)
    }

    /// The tab's page, which what acts on the tab drives.
    pub fn page(&self) -> &Page {
        &self.page
    }

    /// The environment's browser connection.
    pub fn browser(&self) -> &BrowserHandle {
        &self.browser
    }

    /// Cancelled once the tab has closed or its environment ended.
    pub fn ended(&self) -> &CancellationToken {
        &self.ended
    }

    /// What the tab keeps: its operation lock and each command family's data.
    pub fn state(&self) -> &TabState {
        &self.state
    }

    /// How many times the agent showed the tab to the user.
    pub fn shows(&self) -> u64 {
        *self.state.shows.borrow()
    }

    /// Shows the tab to the user once more: its count rises, and the
    /// user's work panel selects it when it learns of the count
    /// (`live-view.md` § Showing a tab). Nothing else changes in the browser.
    pub fn show(&self) {
        self.state.shows.send_modify(|shows| *shows += 1);
        self.state.changes.send_modify(|revision| *revision += 1);
    }

    /// Who opened the tab.
    pub fn created_by(&self) -> &BrowserCreatedBy {
        &self.created_by
    }

    /// Transport identity only; the conversation registry will issue public tab IDs.
    pub fn target_id(&self) -> &str {
        self.page.target_id().as_ref()
    }

    /// The tab's public ID in its conversation.
    pub fn id(&self) -> &TabId {
        &self.id
    }

    /// Closes the tab without running its `beforeunload` hooks; the
    /// registry no longer lists it once this returns. Closing the last tab
    /// retires the environment instead (`browser.md` § Lifetime), which the
    /// environment's owner does when it has emptied.
    pub async fn close(&self, cancellation: &CancellationToken, timeout: Duration) -> Result<()> {
        self.close_request(cancellation, timeout).await.map(|_| ())
    }

    /// Closes the tab as [`Self::close`] does, and says whether the
    /// environment retires with it.
    pub async fn close_request(
        &self,
        cancellation: &CancellationToken,
        timeout: Duration,
    ) -> Result<Closed> {
        let deadline = tokio::time::Instant::now() + timeout;
        let close = self.registry.close(self.page.target_id().clone(), deadline);
        // Not raced with the environment's end: closing the last tab ends the
        // environment, and the close's answer says so. An environment that
        // ends otherwise drops the request, which answers `Closed`.
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => Err(BrowserError::Cancelled),
            closed = tokio::time::timeout_at(deadline, close) => {
                closed.map_err(|_| BrowserError::Timeout)?
            }
        }
    }

    /// A dialog can block acknowledgement of native input; retain cleanup in the tab.
    pub async fn input<T>(
        &self,
        operation: &Operation<'_>,
        input: impl std::future::Future<Output = Result<T>>,
    ) -> Result<T> {
        let mut dialog = self.state.dialog.watch();
        if dialog.borrow().is_some() {
            return Err(BrowserError::DialogBlocked);
        }
        operation
            .run(async {
                tokio::select! {
                    result = input => result,
                    opened = dialog.wait_for(|dialog| dialog.is_some()) => {
                        opened.map_err(|_| BrowserError::Closed)?;
                        Err(BrowserError::DialogBlocked)
                    },
                }
            })
            .await
    }
}
