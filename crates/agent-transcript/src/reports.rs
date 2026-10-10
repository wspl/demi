//! The text the model reads of command reports (`runtime.md` § Command
//! reports), rendered from a `wakeup` block's reports wherever they are
//! replayed or estimated: each names the command by its number and its
//! call's title, says what happened, and carries its output as a `shell`
//! result shows it.

use demi_shared_types::{CommandId, CommandReport, ReportEvent, StoppedBy};

/// A duration as the model reads it: `45s`, `4m`, `1m5s`, `2h` or `1h3m`,
/// rounded to the nearest second.
pub fn duration(ms: u64) -> String {
    let seconds = (ms + 500) / 1000;
    if seconds < 60 {
        return format!("{seconds}s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return match seconds % 60 {
            0 => format!("{minutes}m"),
            rest => format!("{minutes}m{rest}s"),
        };
    }
    match minutes % 60 {
        0 => format!("{}h", minutes / 60),
        rest => format!("{}h{rest}m", minutes / 60),
    }
}

/// The text of reports that arrived together: one paragraph each.
pub fn reports_text(reports: &[CommandReport]) -> String {
    reports
        .iter()
        .map(report_text)
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// The text of one report: what happened, its output, and for a command
/// that still runs, how to change how often it reports.
pub fn report_text(report: &CommandReport) -> String {
    let named = named(&report.command_id, &report.title);
    let command = &report.command_id;
    let headline = match &report.event {
        ReportEvent::Running {
            running_ms,
            idle_ms,
            ..
        } => format!(
            "{named} is still running after {}; no output for {}.",
            duration(*running_ms),
            duration(*idle_ms),
        ),
        ReportEvent::Ended {
            exit_code: Some(code),
        } => format!("{named} ended with exit code {code}."),
        ReportEvent::Ended { exit_code: None } => format!("{named} ended."),
        ReportEvent::Stopped {
            by: Some(StoppedBy::User),
        } => format!("{named} was stopped by the user."),
        ReportEvent::Stopped {
            by: Some(StoppedBy::Agent { number }),
        } => format!("{named} was stopped by agent {number}."),
        ReportEvent::Stopped { by: None } => format!("{named} was stopped."),
        ReportEvent::Lost { reason } => {
            format!("{named} was lost: {reason}. Start it again if it is still needed.")
        }
    };
    let mut lines = vec![headline, labelled_output(&report.output)];
    if let ReportEvent::Running { interval_ms, .. } = report.event {
        lines.push(format!(
            "It reports every {}; change that with demi shell status {command} --interval <duration>, or with --resident to hear only of its end.",
            duration(interval_ms.into())
        ));
    }
    lines.join("\n")
}

/// A command's output after the lines that say what became of it, as a
/// report or a result shows it: labelled, or said to be empty.
pub fn labelled_output(output: &str) -> String {
    if output.is_empty() {
        "output: (empty)".to_owned()
    } else {
        format!("output:\n{output}")
    }
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
