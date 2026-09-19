//! What a viewer does to tabs besides input (`browser-live-view.md` § What
//! the user sees): opening, closing and navigating them, choosing a tab's
//! mode, and answering its dialogs, as the agent's commands do and beside
//! them.

use std::{
    sync::{Arc, Weak},
    time::Duration,
};

use chromiumoxide::cdp::browser_protocol::page::{
    GetNavigationHistoryParams, NavigateParams, NavigateToHistoryEntryParams, ReloadParams,
};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{hub::Membership, writer::Writer};
use crate::browser::{
    BrowserEnvironment, BrowserError, BrowserTab, Result, conversations::Controller,
    navigation::validate_url, operation::CONTROL_TIMEOUT, viewport::Mode,
};

/// How long opening a tab may take before the viewer hears it failed.
const OPEN_TIMEOUT: Duration = Duration::from_secs(30);
/// How long a navigation the viewer started may load.
const LOAD_TIMEOUT: Duration = Duration::from_secs(60);

pub(super) enum Command {
    Open(Option<String>),
    Close(String),
    Navigate { tab: String, url: String },
    History { tab: String, action: String },
    Mode { tab: String, mode: Mode },
}

/// Runs a viewer's commands in order; a tab it opened arrives on the
/// returned queue for the viewer to watch. A command the viewer sent
/// completes even if the view ends first: closing the last tab retires the
/// browser, which must not stop halfway.
pub(super) fn start(
    environment: BrowserEnvironment,
    controller: Arc<Controller>,
    membership: Weak<Membership>,
    writer: Writer,
) -> (mpsc::UnboundedSender<Command>, mpsc::Receiver<BrowserTab>) {
    let (commands, mut receiver) = mpsc::unbounded_channel();
    let (opened, opening) = mpsc::channel(1);
    tokio::spawn(async move {
        while let Some(command) = receiver.recv().await {
            let result = run(&environment, &controller, &membership, command, &opened).await;
            if let Err(error) = result
                && !environment.ended.is_cancelled()
            {
                writer.notice(error.code(), &error.to_string()).await;
            }
        }
    });
    (commands, opening)
}

async fn run(
    environment: &BrowserEnvironment,
    controller: &Controller,
    membership: &Weak<Membership>,
    command: Command,
    opened: &mpsc::Sender<BrowserTab>,
) -> Result<()> {
    let cancel = CancellationToken::new();
    match command {
        Command::Open(url) => {
            let deadline = tokio::time::Instant::now() + OPEN_TIMEOUT;
            let tab = environment
                .open_user(url.as_deref(), &cancel, deadline)
                .await?;
            let _gone = opened.send(tab).await;
        }
        Command::Close(id) => {
            let tab = find(environment, &id).await?;
            tab.close(&cancel, CONTROL_TIMEOUT).await?;
            // As the agent's close: the last tab's close retires the browser.
            controller.retire_empty(environment).await?;
        }
        Command::Navigate { tab, url } => {
            validate_url(&url)?;
            visit(&find(environment, &tab).await?, &url);
        }
        Command::History { tab, action } => {
            let tab = find(environment, &tab).await?;
            if action == "reload" {
                detach(&tab, ReloadParams::default());
                return Ok(());
            }
            let history = tab
                .page
                .execute(GetNavigationHistoryParams {})
                .await?
                .result;
            let index = history.current_index + if action == "back" { -1 } else { 1 };
            let entry = usize::try_from(index)
                .ok()
                .and_then(|index| history.entries.get(index))
                .ok_or(BrowserError::HistoryBoundary)?;
            detach(&tab, NavigateToHistoryEntryParams::new(entry.id));
        }
        Command::Mode { tab, mode } => {
            if let Some(membership) = membership.upgrade() {
                membership
                    .mode(&find(environment, &tab).await?, mode)
                    .await?;
            }
        }
    }
    Ok(())
}

/// The tab with public ID `id`.
pub(super) async fn find(environment: &BrowserEnvironment, id: &str) -> Result<BrowserTab> {
    environment
        .tabs(&CancellationToken::new(), CONTROL_TIMEOUT)
        .await?
        .into_iter()
        .find(|tab| tab.id() == id)
        .ok_or(BrowserError::TabNotFound)
}

/// Starts loading `url` in `tab` as an address bar does. The viewer sees the
/// page load, or Chrome's own error page, so nothing waits for it.
pub(in crate::browser) fn visit(tab: &BrowserTab, url: &str) {
    detach(tab, NavigateParams::new(url));
}

/// Sends a navigation without waiting for it: chromiumoxide answers one
/// only once the page loaded.
fn detach<C>(tab: &BrowserTab, command: C)
where
    C: chromiumoxide::types::Command + Send + 'static,
    C::Response: Send,
{
    let page = tab.page.clone();
    tokio::spawn(async move {
        let _loaded = tokio::time::timeout(LOAD_TIMEOUT, page.execute(command)).await;
    });
}

/// Answers the tab's dialog for the viewer, unless someone answered first.
pub(super) fn answer(tab: BrowserTab, accept: bool, text: Option<String>, writer: Writer) {
    tokio::spawn(async move {
        let Some(dialog) = tab.state.dialog.borrow().clone() else {
            return;
        };
        // Only a prompt takes text.
        let text = text.filter(|_| dialog.r#type.as_ref() == "prompt" && accept);
        if let Err(error) = tab.answer_dialog(&dialog, accept, text).await
            && tab.state.dialog.borrow().is_some()
        {
            writer.notice(error.code(), &error.to_string()).await;
        }
    });
}
