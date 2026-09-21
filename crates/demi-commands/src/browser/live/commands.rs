//! What a viewer does to the tab it watches besides input
//! (`browser-live-view.md` § The stream): choosing its mode and answering its
//! dialogs, as the agent's commands do and beside them.

use std::sync::Weak;

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use super::{hub::Membership, writer::Writer};
use crate::browser::{
    BrowserEnvironment, BrowserError, BrowserTab, Result, operation::CONTROL_TIMEOUT,
    viewport::Mode,
};

pub(super) enum Command {
    Mode { tab: String, mode: Mode },
}

/// Runs a viewer's commands in order, beside its input and pictures.
pub(super) fn start(
    environment: BrowserEnvironment,
    membership: Weak<Membership>,
    writer: Writer,
) -> mpsc::UnboundedSender<Command> {
    let (commands, mut receiver) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        while let Some(command) = receiver.recv().await {
            let result = run(&environment, &membership, command).await;
            if let Err(error) = result
                && !environment.ended.is_cancelled()
            {
                writer.notice(error.code(), &error.to_string()).await;
            }
        }
    });
    commands
}

async fn run(
    environment: &BrowserEnvironment,
    membership: &Weak<Membership>,
    command: Command,
) -> Result<()> {
    match command {
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
