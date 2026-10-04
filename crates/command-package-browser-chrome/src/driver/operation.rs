use std::{future::Future, time::Duration};

use chromiumoxide::error::CdpError;
use thiserror::Error;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use demi_command_package_browser_protocol::browser::{
    ActionProgress, AssetsExportResult, BrowserErrorCode, ErrorDetails,
};
use demi_command_protocol::MAX_MEDIUM_BYTES;

pub const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);

pub type Result<T> = std::result::Result<T, BrowserError>;

#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("unsupported browser capability: {0}")]
    UnsupportedCapability(String),
    #[error("browser asset inventory is stale")]
    StaleInventory,
    #[error("browser tool declarations are stale")]
    StaleTools,
    #[error("browser cursor is stale or belongs to another stream")]
    StaleCursor,
    #[error("CDP method is denied: {0}")]
    CdpMethodDenied(String),
    #[error("some browser items failed")]
    PartialFailure { export: AssetsExportResult },
    #[error("read-only evaluation rejected a possible side effect")]
    SideEffectRejected,
    #[error("browser environment is closed")]
    Closed,
    #[error("{0}")]
    Connection(String),
    #[error("browser input outcome is unknown: {source}")]
    OutcomeUnknown { source: Box<BrowserError> },
    #[error("browser tab was not found")]
    TabNotFound,
    #[error("browser target matched no elements")]
    TargetNotFound,
    #[error("element condition failed: {condition}; interceptor: {interceptor:?}")]
    NotActionable {
        condition: String,
        interceptor: Option<String>,
    },
    #[error("no navigation entry in that direction")]
    HistoryBoundary,
    #[error("main document navigation failed: {0}")]
    NavigationFailed(String),
    #[error("output already exists: {0}")]
    OutputExists(String),
    #[error("result exceeds the browser output limit")]
    ResultTooLarge,
    /// A screenshot of more bytes than a medium can be, which the command
    /// would return as one (`browser.md` § Images and large outputs).
    #[error(
        "the screenshot is {0} bytes, more than the {MAX_MEDIUM_BYTES} a command returns as an image; save it to a file with --output <file>"
    )]
    ScreenshotTooLarge(usize),
    /// A CDP response over the connection's limit, which failed only the
    /// request it answers (`browser.md` § Native driver).
    #[error("Chrome's answer is too large: {0}")]
    MessageTooLarge(String),
    /// A capture whose CDP response is over the connection's limit, so
    /// that even `--output` cannot keep it (`browser.md` § Images and large
    /// outputs).
    #[error(
        "the screenshot is too large for Chrome to send: {0}; capture a part of the page with --clip x,y,width,height"
    )]
    CaptureTooLarge(String),
    #[error("password values are protected")]
    ProtectedValue,
    #[error("browser could not start: {0}")]
    Unavailable(String),
    /// Chrome refused to start as root on Linux (`browser.md` § Native driver).
    #[error(
        "Chrome does not run as root on Linux with its sandbox, which Demi keeps: run the runner as an ordinary user"
    )]
    Root,
    /// Chrome for Testing could not be installed or found intact.
    #[error("Chrome for Testing {0}")]
    Installation(String),
    #[error("{source}")]
    Action {
        source: Box<BrowserError>,
        details: Box<ErrorDetails>,
    },
    #[error("browser operation was cancelled")]
    Cancelled,
    #[error("browser operation exceeded its deadline")]
    Timeout,
    #[error("another operation owns this browser tab")]
    Busy,
    #[error(
        "browser action is blocked by a JavaScript dialog; inspect and handle the dialog before continuing"
    )]
    DialogBlocked,
    #[error("the browser tab has no JavaScript dialog")]
    DialogNotFound,
    #[error("this action is not valid for the current JavaScript dialog")]
    InvalidDialogAction,
    #[error("browser target matched {0} elements; exactly one is required")]
    Ambiguous(usize),
    #[error("browser node reference is stale or belongs to another tab")]
    StaleReference,
    #[error("browser result is not representable as JSON: {0}")]
    InvalidResult(String),
    #[error("invalid browser configuration: {0}")]
    Configuration(String),
    #[error(transparent)]
    Cdp(CdpError),
    #[error(transparent)]
    Events(#[from] chromiumoxide::listeners::EventStreamError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("browser profile retained at {path}: {source}")]
    ProfileRetained {
        path: std::path::PathBuf,
        source: Box<BrowserError>,
    },
    #[error("browser event task failed: {0}")]
    Task(#[from] tokio::task::JoinError),
    #[error("browser cleanup failed: {cleanup}; preceding operation: {operation:?}")]
    Cleanup {
        operation: Option<Box<BrowserError>>,
        cleanup: Box<BrowserError>,
    },
}

/// One browser operation shares a deadline across every wait and CDP step.
pub struct Operation<'a> {
    ended: &'a CancellationToken,
    cancelled: &'a CancellationToken,
    deadline: Instant,
    /// How far the action's input got. A std mutex: set and read in
    /// passing, never across an await.
    progress: std::sync::Mutex<ActionProgress>,
    connection_failure: Option<&'a tokio::sync::watch::Sender<Option<String>>>,
}

impl<'a> Operation<'a> {
    pub fn new(
        ended: &'a CancellationToken,
        cancelled: &'a CancellationToken,
        timeout: Duration,
    ) -> Self {
        Self::until(ended, cancelled, Instant::now() + timeout)
    }

    pub fn until(
        ended: &'a CancellationToken,
        cancelled: &'a CancellationToken,
        deadline: Instant,
    ) -> Self {
        Self {
            ended,
            cancelled,
            deadline,
            progress: std::sync::Mutex::new(ActionProgress::NotStarted),
            connection_failure: None,
        }
    }

    /// Reports `failure`, the browser transport's cause, when the lifetime
    /// the operation runs in ends mid-command.
    pub fn reporting(mut self, failure: &'a tokio::sync::watch::Sender<Option<String>>) -> Self {
        self.connection_failure = Some(failure);
        self
    }

    /// The input is on its way: its outcome is unknown until it is
    /// delivered.
    pub fn begin_input(&self) {
        self.set_progress(ActionProgress::Unknown);
    }

    pub fn input_not_delivered(&self) {
        self.set_progress(ActionProgress::NotStarted);
    }

    pub fn complete_input(&self) {
        self.set_progress(ActionProgress::Completed);
    }

    fn set_progress(&self, progress: ActionProgress) {
        *self.progress.lock().expect("the input progress is intact") = progress;
    }

    pub fn failure(&self, error: BrowserError, tab: &str, url: Option<&str>) -> BrowserError {
        let progress = *self.progress.lock().expect("the input progress is intact");
        let error = if progress != ActionProgress::NotStarted && error.is_connection_loss() {
            BrowserError::OutcomeUnknown {
                source: Box::new(error),
            }
        } else {
            error
        };
        let mut details = error.details();
        details.action.get_or_insert(progress);
        details.tab.get_or_insert_with(|| tab.to_owned());
        if details.url.is_none() {
            details.url = url.map(str::to_owned);
        }
        BrowserError::Action {
            source: Box::new(error),
            details: Box::new(details),
        }
    }

    pub async fn run<T>(&self, future: impl Future<Output = Result<T>>) -> Result<T> {
        tokio::select! {
            biased;
            _ = self.ended.cancelled() => Err(self.connection_failure
                .and_then(|failure| failure.borrow().clone())
                .map(BrowserError::Connection)
                .unwrap_or(BrowserError::Closed)),
            _ = self.cancelled.cancelled() => Err(if Instant::now() >= self.deadline {
                // The conversation controller cancels at this same shared deadline.
                BrowserError::Timeout
            } else {
                BrowserError::Cancelled
            }),
            result = tokio::time::timeout_at(self.deadline, future) => {
                result.map_err(|_| BrowserError::Timeout)?
            }
        }
    }
}

/// The number of the agent a browser command acts for: tabs, debugging
/// sessions and temporary tabs belong to it (`native-runtime.md` § Command
/// context).
pub fn agent(context: &demi_command_sdk::InvocationContext) -> Result<u64> {
    context
        .request
        .context
        .caller
        .agent_number()
        .ok_or_else(|| BrowserError::Configuration("browser commands act for an agent".into()))
}

/// Preserve the browser operation error when its required cleanup also fails.
pub fn after_cleanup<T>(operation: Result<T>, cleanup: Result<()>) -> Result<T> {
    match (operation, cleanup) {
        (result, Ok(())) => result,
        (result, Err(cleanup)) => Err(BrowserError::Cleanup {
            operation: result.err().map(Box::new),
            cleanup: Box::new(cleanup),
        }),
    }
}

impl BrowserError {
    /// Preserve transport loss through browser cleanup and action context wrappers.
    fn is_connection_loss(&self) -> bool {
        match self {
            Self::Connection(_) => true,
            Self::Action { source, .. } | Self::ProfileRetained { source, .. } => {
                source.is_connection_loss()
            }
            Self::Cleanup {
                operation: Some(error),
                ..
            } => error.is_connection_loss(),
            Self::Cleanup { cleanup, .. } => cleanup.is_connection_loss(),
            _ => false,
        }
    }

    /// Preserve the browser condition that exhausted a shared readiness deadline.
    pub fn with_deadline_cause(self, cause: BrowserError) -> Self {
        match self {
            Self::Cleanup { operation, cleanup } => Self::Cleanup {
                operation: Some(Box::new(match operation {
                    Some(error) => error.with_deadline_cause(cause),
                    None => cause,
                })),
                cleanup,
            },
            _ => cause,
        }
    }

    pub fn is_deadline(&self) -> bool {
        self.code() == BrowserErrorCode::Timeout
    }

    pub fn code(&self) -> BrowserErrorCode {
        match self {
            Self::Action { source, .. } => source.code(),
            Self::Cleanup {
                operation: Some(error),
                ..
            } => error.code(),
            Self::Cleanup { cleanup, .. } => cleanup.code(),
            Self::ProfileRetained { source, .. } => source.code(),
            Self::Closed | Self::Connection(_) => BrowserErrorCode::BrowserLost,
            Self::OutcomeUnknown { .. } => BrowserErrorCode::OutcomeUnknown,
            Self::TabNotFound => BrowserErrorCode::TabNotFound,
            Self::StaleInventory => BrowserErrorCode::StaleInventory,
            Self::StaleTools => BrowserErrorCode::StaleTools,
            Self::StaleCursor => BrowserErrorCode::StaleCursor,
            Self::SideEffectRejected => BrowserErrorCode::SideEffectRejected,
            Self::CdpMethodDenied(_) => BrowserErrorCode::CdpMethodDenied,
            Self::PartialFailure { .. } => BrowserErrorCode::PartialFailure,
            Self::TargetNotFound => BrowserErrorCode::TargetNotFound,
            Self::NotActionable { .. } => BrowserErrorCode::NotActionable,
            Self::HistoryBoundary => BrowserErrorCode::HistoryBoundary,
            Self::NavigationFailed(_) => BrowserErrorCode::NavigationFailed,
            Self::OutputExists(_) => BrowserErrorCode::OutputExists,
            Self::ResultTooLarge
            | Self::ScreenshotTooLarge(_)
            | Self::MessageTooLarge(_)
            | Self::CaptureTooLarge(_) => BrowserErrorCode::ResultTooLarge,
            Self::UnsupportedCapability(_) => BrowserErrorCode::UnsupportedCapability,
            Self::ProtectedValue => BrowserErrorCode::ProtectedValue,
            Self::Unavailable(_) | Self::Installation(_) | Self::Root => {
                BrowserErrorCode::BrowserUnavailable
            }
            Self::Cancelled => BrowserErrorCode::Cancelled,
            Self::Timeout => BrowserErrorCode::Timeout,
            Self::Busy => BrowserErrorCode::TabBusy,
            Self::DialogBlocked => BrowserErrorCode::DialogBlocked,
            Self::DialogNotFound => BrowserErrorCode::DialogNotFound,
            Self::InvalidDialogAction => BrowserErrorCode::InvalidDialogAction,
            Self::Ambiguous(_) => BrowserErrorCode::AmbiguousTarget,
            Self::StaleReference => BrowserErrorCode::StaleRef,
            Self::InvalidResult(_) => BrowserErrorCode::UnsupportedResult,
            Self::Configuration(_) => BrowserErrorCode::InvalidInput,
            Self::Io(_) => BrowserErrorCode::IoError,
            Self::Cdp(_) | Self::Events(_) | Self::Task(_) => BrowserErrorCode::DriverError,
        }
    }

    pub fn details(&self) -> ErrorDetails {
        match self {
            Self::Action { details, .. } => details.as_ref().clone(),
            Self::PartialFailure { export } => ErrorDetails {
                export: Some(export.clone()),
                ..ErrorDetails::default()
            },
            Self::OutcomeUnknown { source } => ErrorDetails {
                action: Some(ActionProgress::Unknown),
                ..source.details()
            },
            Self::Cleanup {
                operation: Some(error),
                ..
            } => error.details(),
            Self::NotActionable {
                condition,
                interceptor,
            } => ErrorDetails {
                condition: Some(condition.clone()),
                interceptor: interceptor.clone(),
                ..ErrorDetails::default()
            },
            Self::Ambiguous(count) => ErrorDetails {
                count: Some(*count),
                ..ErrorDetails::default()
            },
            _ => ErrorDetails::default(),
        }
    }
}

impl From<CdpError> for BrowserError {
    fn from(error: CdpError) -> Self {
        match error {
            CdpError::Ws(_) | CdpError::ChannelSendError(_) | CdpError::NoResponse => {
                Self::Connection(error.to_string())
            }
            CdpError::Timeout => Self::Timeout,
            // Chrome says so only on its standard error, and exits: the
            // launch reports the exit, or the end of the output, with what
            // Chrome wrote before it.
            CdpError::LaunchExit(_, ref stderr) | CdpError::LaunchIo(_, ref stderr)
                if String::from_utf8_lossy(stderr.as_slice())
                    .contains("Running as root without --no-sandbox is not supported") =>
            {
                Self::Root
            }
            CdpError::Chrome(error) if error.code == chromiumoxide::conn::MESSAGE_TOO_LARGE => {
                Self::MessageTooLarge(error.message)
            }
            // Chrome provides only a message for a closed target session.
            CdpError::Chrome(ref error) if error.message == "Session with given id not found." => {
                Self::TabNotFound
            }
            _ => Self::Cdp(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_cause_survives_cleanup_wrappers() {
        let timed_out = after_cleanup::<()>(Err(BrowserError::Timeout), Err(BrowserError::Timeout))
            .unwrap_err();
        assert!(timed_out.is_deadline());
        let failure = timed_out.with_deadline_cause(BrowserError::NotActionable {
            condition: "hit".into(),
            interceptor: Some("overlay".into()),
        });
        assert_eq!(failure.code(), BrowserErrorCode::NotActionable);
        assert_eq!(failure.details().condition.as_deref(), Some("hit"));
        assert_eq!(failure.details().interceptor.as_deref(), Some("overlay"));
        assert!(
            matches!(failure, BrowserError::Cleanup { cleanup, .. } if matches!(*cleanup, BrowserError::Timeout))
        );
    }

    #[tokio::test]
    async fn controller_deadline_cancellation_is_a_timeout() {
        let ended = CancellationToken::new();
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        let expired = Operation::until(&ended, &cancelled, Instant::now());
        assert!(matches!(
            expired.run(std::future::pending::<Result<()>>()).await,
            Err(BrowserError::Timeout)
        ));
        let active = Operation::new(&ended, &cancelled, CONTROL_TIMEOUT);
        assert!(matches!(
            active.run(std::future::pending::<Result<()>>()).await,
            Err(BrowserError::Cancelled)
        ));
    }

    #[tokio::test]
    async fn transport_end_retains_its_cause_while_explicit_close_stays_closed() {
        let ended = CancellationToken::new();
        let cancelled = CancellationToken::new();
        let (failure, _) = tokio::sync::watch::channel(None);
        let mut operation = Operation::new(&ended, &cancelled, CONTROL_TIMEOUT);
        operation.connection_failure = Some(&failure);
        operation.begin_input();
        ended.cancel();
        let closed = operation
            .run(std::future::pending::<Result<()>>())
            .await
            .unwrap_err();
        assert_eq!(
            operation.failure(closed, "tab", None).code(),
            BrowserErrorCode::BrowserLost
        );
        failure.send_replace(Some("transport ended".into()));
        let lost = operation
            .run(std::future::pending::<Result<()>>())
            .await
            .unwrap_err();
        let error = operation.failure(lost, "tab", None);
        assert_eq!(error.code(), BrowserErrorCode::OutcomeUnknown);
        assert_eq!(error.details().action, Some(ActionProgress::Unknown));
    }

    #[test]
    fn connection_loss_after_dispatch_has_an_unknown_outcome() {
        let ended = CancellationToken::new();
        let cancelled = CancellationToken::new();
        let operation = Operation::new(&ended, &cancelled, CONTROL_TIMEOUT);
        let lost = || BrowserError::Connection("transport disconnected".into());
        let before = operation.failure(lost(), "tab", Some("https://example.test"));
        assert_eq!(before.code(), BrowserErrorCode::BrowserLost);
        assert_eq!(before.details().action, Some(ActionProgress::NotStarted));
        operation.begin_input();
        for completed in [false, true] {
            if completed {
                operation.complete_input();
            }
            let cause = after_cleanup::<()>(Err(lost()), Err(BrowserError::Closed)).unwrap_err();
            let after = operation.failure(cause, "tab", Some("https://example.test"));
            assert_eq!(after.code(), BrowserErrorCode::OutcomeUnknown);
            assert_eq!(
                after.details(),
                ErrorDetails {
                    action: Some(ActionProgress::Unknown),
                    tab: Some("tab".into()),
                    url: Some("https://example.test".into()),
                    ..ErrorDetails::default()
                }
            );
            assert!(after.to_string().contains("transport disconnected"));
            assert_eq!(
                operation.failure(BrowserError::Closed, "tab", None).code(),
                BrowserErrorCode::BrowserLost
            );
            assert_eq!(
                operation
                    .failure(BrowserError::Cancelled, "tab", None)
                    .code(),
                BrowserErrorCode::Cancelled
            );
        }
    }

    #[test]
    fn browser_errors_keep_causes_and_input_progress() {
        for (error, code) in [
            (BrowserError::TabNotFound, "tab_not_found"),
            (BrowserError::Closed, "browser_lost"),
            (BrowserError::TargetNotFound, "target_not_found"),
            (BrowserError::Ambiguous(2), "ambiguous_target"),
            (BrowserError::HistoryBoundary, "history_boundary"),
            (
                BrowserError::NavigationFailed("ERR_EMPTY_RESPONSE".into()),
                "navigation_failed",
            ),
            (
                BrowserError::OutputExists("existing.png".into()),
                "output_exists",
            ),
            (
                BrowserError::Io(std::io::Error::from(std::io::ErrorKind::PermissionDenied)),
                "io_error",
            ),
            (
                BrowserError::Configuration("bad key".into()),
                "invalid_input",
            ),
        ] {
            assert_eq!(error.code().to_string(), code);
        }
        let ended = CancellationToken::new();
        let cancelled = CancellationToken::new();
        let operation = Operation::new(&ended, &cancelled, CONTROL_TIMEOUT);
        let progress = |error| operation.failure(error, "tab", None).details().action;
        assert_eq!(
            progress(BrowserError::Timeout),
            Some(ActionProgress::NotStarted)
        );
        operation.begin_input();
        assert_eq!(
            progress(BrowserError::Closed),
            Some(ActionProgress::Unknown)
        );
        operation.complete_input();
        assert_eq!(
            progress(BrowserError::Timeout),
            Some(ActionProgress::Completed)
        );
    }
}
