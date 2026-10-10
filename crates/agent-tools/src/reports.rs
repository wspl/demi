//! What a command's report tells its node (`runtime.md` § Command reports):
//! its progress while it runs, or its end, each with the output since the
//! node's last look as a `shell` result shows it. A report is data; the text
//! the model reads is rendered from it where it is replayed
//! ([`demi_agent_transcript::report_text`]).

use demi_agent_transcript::{REPLAY_CHARS, report_text};
use demi_host_interface::CommandStatus;
use demi_shared_types::{CommandId, CommandReport, ReportEvent, StoppedBy};

use crate::{Stopper, result::report_output};

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

/// The report of `status`, a command that still runs and reports every
/// `interval_ms`: how long it has run and printed nothing, and what a look
/// shows of its output.
pub fn progress_report(status: &CommandStatus, title: &str, interval_ms: u32) -> CommandReport {
    let event = ReportEvent::Running {
        running_ms: status.running_ms,
        idle_ms: status.idle_ms,
        interval_ms,
    };
    report(&status.command_id, title, event, Some(status))
}

/// The report of `command`'s end, `end`, with what it printed since the
/// node's last look, as `status` shows it once it ended; no output when its
/// output is not known.
pub fn end_report(
    command: &CommandId,
    title: &str,
    end: EndOf,
    status: Option<&CommandStatus>,
) -> CommandReport {
    let event = match end {
        EndOf::Exited(code) => ReportEvent::Ended {
            exit_code: Some(code),
        },
        EndOf::Stopped(Some(Stopper::User)) => ReportEvent::Stopped {
            by: Some(StoppedBy::User),
        },
        EndOf::Stopped(Some(Stopper::Agent(number))) => ReportEvent::Stopped {
            by: Some(StoppedBy::Agent { number }),
        },
        EndOf::Stopped(Some(Stopper::Itself) | None) => ReportEvent::Stopped { by: None },
        EndOf::Lost => ReportEvent::lost_with_connection(),
        EndOf::Unrecorded => ReportEvent::Ended { exit_code: None },
    };
    report(command, title, event, status)
}

/// A report of `event` whose output, read from `status`, takes what the
/// replay bound leaves after the report's other lines, so that replay sends
/// it unchanged (`runtime.md` § Results and previews).
fn report(
    command: &CommandId,
    title: &str,
    event: ReportEvent,
    status: Option<&CommandStatus>,
) -> CommandReport {
    let mut report = CommandReport {
        command_id: command.clone(),
        title: title.to_owned(),
        event,
        output: String::new(),
    };
    if let Some(status) = status {
        let others = report_text(&report).chars().count();
        report.output = report_output(status, REPLAY_CHARS.saturating_sub(others));
    }
    report
}
