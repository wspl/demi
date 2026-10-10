//! What a command's report tells its node (`runtime.md` § Command reports):
//! its progress while it runs, or its end, each with the output since the
//! node's last look as a `shell` result shows it. A report is data; the text
//! the model reads is rendered from it where it is replayed
//! ([`demi_agent_transcript::report_text`]). Its output moves the node's
//! place only when the report is written into the transcript, so a report
//! dropped because a look showed the end first moves nothing.

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

/// The report of `command`, which still runs and reports every
/// `interval_ms`: how long it has run and printed nothing. Its output is
/// read when the report is written ([`fill_output`]), which moves the
/// node's place then.
pub fn progress_report(
    command: &CommandId,
    title: &str,
    running_ms: u64,
    idle_ms: u64,
    interval_ms: u32,
) -> CommandReport {
    report(
        command,
        title,
        ReportEvent::Running {
            running_ms,
            idle_ms,
            interval_ms,
        },
    )
}

/// The report of `command`'s end, `end`; its output is read from
/// `status` once it ended, or when the report is written ([`fill_output`]).
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
    let mut report = report(command, title, event);
    if let Some(status) = status {
        fill_output(&mut report, status);
    }
    report
}

/// Gives `report` the output `status` shows since the node's last look,
/// within what the replay bound leaves after the report's other lines, so
/// that replay sends it unchanged (`runtime.md` § Results and previews).
pub fn fill_output(report: &mut CommandReport, status: &CommandStatus) {
    report.output.clear();
    let others = report_text(report).chars().count();
    report.output = report_output(status, REPLAY_CHARS.saturating_sub(others));
}

/// A report of `event`, without output yet.
fn report(command: &CommandId, title: &str, event: ReportEvent) -> CommandReport {
    CommandReport {
        command_id: command.clone(),
        title: title.to_owned(),
        event,
        output: String::new(),
    }
}
