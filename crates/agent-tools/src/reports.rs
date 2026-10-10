//! What a command's report tells its node (`runtime.md` § Command reports):
//! its progress while it runs, as a look shows it, with how to change how
//! often it reports; or its end. Each names the command by its number and
//! the title of the call that started it, since a node may have several
//! running.

use demi_host_interface::CommandStatus;
use demi_shared_types::CommandId;

use crate::{Look, Stopper, duration, look_text};

/// How a command ended, as its end's report tells it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndOf {
    Exited(i32),
    /// It was stopped, by whom when that is known.
    Stopped(Option<Stopper>),
    /// It ended with its Host's connection.
    Lost,
    /// Its record keeps no end.
    Unrecorded,
}

/// The command as a report names it: `Command 17 (Run the test suite)`, or
/// `Command 17` when its call's title is not known.
fn named(command: &CommandId, title: &str) -> String {
    if title.is_empty() {
        format!("Command {command}")
    } else {
        format!("Command {command} ({title})")
    }
}

/// The report of `status`, a command that still runs and reports every
/// `interval_ms`: how long it has run and printed nothing, what a look
/// shows of it, and how to change how often it reports.
pub fn progress_report(status: &CommandStatus, title: &str, interval_ms: u32) -> String {
    let command = &status.command_id;
    let look = look_text(
        status,
        Look {
            interval_ms: Some(Some(interval_ms)),
            report: true,
            ..Look::default()
        },
    );
    [
        format!(
            "{} is still running after {}; no output for {}.",
            named(command, title),
            duration(status.running_ms),
            duration(status.idle_ms),
        ),
        look,
        format!(
            "It reports every {}; change that with demi shell status {command} --interval <duration>, or with --resident to hear only of its end.",
            duration(interval_ms.into())
        ),
    ]
    .join("\n")
}

/// The report of `command`'s end, `end`.
pub fn end_report(command: &CommandId, title: &str, end: EndOf) -> String {
    let named = named(command, title);
    match end {
        EndOf::Exited(code) => format!(
            "{named} ended with exit code {code}; look at it with demi shell status {command}."
        ),
        EndOf::Stopped(Some(Stopper::User)) => format!("{named} was stopped by the user."),
        EndOf::Stopped(Some(Stopper::Agent(agent))) => {
            format!("{named} was stopped by agent {agent}.")
        }
        EndOf::Stopped(Some(Stopper::Itself) | None) => format!("{named} was stopped."),
        EndOf::Lost => format!(
            "{named} was lost with its Host's connection. Start it again if it is still needed."
        ),
        EndOf::Unrecorded => {
            format!("{named} ended; look at it with demi shell status {command}.")
        }
    }
}
