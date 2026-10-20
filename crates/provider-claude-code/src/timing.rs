//! How long a run waited for its process and its first word
//! (`crates/provider-claude-code/src/prepared.rs`).
//!
//! The run's span carries:
//!
//! - whether it took a kept, a prepared or a new process;
//! - for a new or prepared one, the steps of its start;
//! - counted from the request, when the process was ready for the request's
//!   input and when the first content delta came.
//!
//! The run logs them at `info` when it ends, so the time to a first word can be
//! read from a backend's log without the wire trace.

use std::time::Duration;

use tokio::time::Instant;
use tracing::Span;

use crate::placement::StartSteps;

/// Which process a run took.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Took {
    /// The process the runtime kept from the session's previous run.
    Kept,
    /// A process started ahead of the request.
    Prepared,
    /// A process started for the request.
    New,
}

impl Took {
    fn as_str(self) -> &'static str {
        match self {
            Self::Kept => "kept",
            Self::Prepared => "prepared",
            Self::New => "new",
        }
    }
}

/// How a process's start went, which the first run that takes the process
/// reports: the placement's steps, building the session file, and the
/// `initialize` exchange.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct StartTiming {
    pub(crate) steps: StartSteps,
    pub(crate) session_build: Duration,
    pub(crate) initialize: Option<Duration>,
}

/// The span of a run, with the fields its timing fills in.
pub(crate) fn run_span(provider: &str, session: &str, request: &str) -> Span {
    tracing::info_span!(
        "claude_code_run",
        provider = %provider,
        session = %session,
        request = %request,
        process = tracing::field::Empty,
        admit_ms = tracing::field::Empty,
        executable_ms = tracing::field::Empty,
        directories_ms = tracing::field::Empty,
        system_ms = tracing::field::Empty,
        session_build_ms = tracing::field::Empty,
        session_write_ms = tracing::field::Empty,
        session_bytes = tracing::field::Empty,
        spawn_ms = tracing::field::Empty,
        initialize_ms = tracing::field::Empty,
        ready_ms = tracing::field::Empty,
        first_delta_ms = tracing::field::Empty,
    )
}

/// One run's timing, counted from its request. Dropping it, however the
/// run ends, logs the span's fields.
pub(crate) struct RunTiming {
    span: Span,
    start: Instant,
    first_delta: bool,
}

impl RunTiming {
    pub(crate) fn new(span: Span) -> Self {
        Self {
            span,
            start: Instant::now(),
            first_delta: false,
        }
    }

    /// The run took a process as `took` says, whose start went as `start`
    /// says when the run is the first to take it.
    pub(crate) fn took(&self, took: Took, start: Option<StartTiming>) {
        self.span.record("process", took.as_str());
        let Some(start) = start else {
            return;
        };
        let steps = start.steps;
        self.span.record("admit_ms", millis(steps.admit));
        self.span.record("executable_ms", millis(steps.executable));
        self.span.record("directories_ms", millis(steps.directories));
        self.span.record("system_ms", millis(steps.system));
        self.span.record("session_build_ms", millis(start.session_build));
        self.span.record("session_write_ms", millis(steps.files));
        self.span.record("session_bytes", steps.file_bytes);
        self.span.record("spawn_ms", millis(steps.spawn));
        if let Some(initialize) = start.initialize {
            self.span.record("initialize_ms", millis(initialize));
        }
    }

    /// The process is ready for the request's input.
    pub(crate) fn ready(&self) {
        self.span.record("ready_ms", millis(self.start.elapsed()));
    }

    /// A content delta reached the agent; the first one counts.
    pub(crate) fn delta(&mut self) {
        if !self.first_delta {
            self.first_delta = true;
            self.span.record("first_delta_ms", millis(self.start.elapsed()));
        }
    }
}

impl Drop for RunTiming {
    fn drop(&mut self) {
        self.span.in_scope(|| tracing::info!("Claude Code run ended"));
    }
}

fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}
