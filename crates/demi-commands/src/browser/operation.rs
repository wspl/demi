use std::{
    future::Future,
    sync::atomic::{AtomicU8, Ordering},
    time::Duration,
};

use chromiumoxide::error::CdpError;
use thiserror::Error;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub(super) const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);

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
    PartialFailure { details: serde_json::Value },
    #[error("read-only evaluation rejected a possible side effect")]
    SideEffectRejected,
    #[error("browser environment is closed")]
    Closed,
    #[error("{0}")]
    Connection(String),
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
    #[error("password values are protected")]
    ProtectedValue,
    #[error("browser could not start: {0}")]
    Unavailable(String),
    #[error("{source}")]
    Action {
        source: Box<BrowserError>,
        details: serde_json::Value,
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
pub(super) struct Operation<'a> {
    ended: &'a CancellationToken,
    cancelled: &'a CancellationToken,
    deadline: Instant,
    progress: AtomicU8,
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
            progress: AtomicU8::new(0),
        }
    }

    pub fn begin_input(&self) {
        self.progress.store(1, Ordering::SeqCst);
    }

    pub fn input_not_delivered(&self) {
        self.progress.store(0, Ordering::SeqCst);
    }

    pub fn complete_input(&self) {
        self.progress.store(2, Ordering::SeqCst);
    }

    pub fn failure(&self, error: BrowserError, tab: &str, url: Option<&str>) -> BrowserError {
        let action = match self.progress.load(Ordering::SeqCst) {
            0 => "not_started",
            2 => "completed",
            _ => "unknown",
        };
        let mut details = error.details();
        if details.get("action").is_none() {
            details["action"] = serde_json::json!(action);
        }
        if details.get("tab").is_none() {
            details["tab"] = serde_json::json!(tab);
        }
        if details.get("url").is_none()
            && let Some(url) = url
        {
            details["url"] = serde_json::json!(url);
        }
        BrowserError::Action {
            source: Box::new(error),
            details,
        }
    }

    pub async fn run<T>(&self, future: impl Future<Output = Result<T>>) -> Result<T> {
        tokio::select! {
            biased;
            _ = self.ended.cancelled() => Err(BrowserError::Closed),
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

/// Preserve the browser operation error when its required cleanup also fails.
pub(super) fn after_cleanup<T>(operation: Result<T>, cleanup: Result<()>) -> Result<T> {
    match (operation, cleanup) {
        (result, Ok(())) => result,
        (result, Err(cleanup)) => Err(BrowserError::Cleanup {
            operation: result.err().map(Box::new),
            cleanup: Box::new(cleanup),
        }),
    }
}

impl BrowserError {
    /// Preserve the browser condition that exhausted a shared readiness deadline.
    pub(super) fn with_deadline_cause(self, cause: BrowserError) -> Self {
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

    pub(super) fn is_deadline(&self) -> bool {
        self.code() == "timeout"
    }

    pub fn code(&self) -> &'static str {
        match self {
            Self::Action { source, .. } => source.code(),
            Self::Cleanup {
                operation: Some(error),
                ..
            } => error.code(),
            Self::Cleanup { cleanup, .. } => cleanup.code(),
            Self::ProfileRetained { source, .. } => source.code(),
            Self::Closed | Self::Connection(_) => "browser_lost",
            Self::TabNotFound => "tab_not_found",
            Self::StaleInventory => "stale_inventory",
            Self::StaleTools => "stale_tools",
            Self::StaleCursor => "stale_cursor",
            Self::SideEffectRejected => "side_effect_rejected",
            Self::CdpMethodDenied(_) => "cdp_method_denied",
            Self::PartialFailure { .. } => "partial_failure",
            Self::TargetNotFound => "target_not_found",
            Self::NotActionable { .. } => "not_actionable",
            Self::HistoryBoundary => "history_boundary",
            Self::NavigationFailed(_) => "navigation_failed",
            Self::OutputExists(_) => "output_exists",
            Self::ResultTooLarge => "result_too_large",
            Self::UnsupportedCapability(_) => "unsupported_capability",
            Self::ProtectedValue => "protected_value",
            Self::Unavailable(_) => "browser_unavailable",
            Self::Cancelled => "cancelled",
            Self::Timeout => "timeout",
            Self::Busy => "tab_busy",
            Self::DialogBlocked => "dialog_blocked",
            Self::DialogNotFound => "dialog_not_found",
            Self::InvalidDialogAction => "invalid_dialog_action",
            Self::Ambiguous(_) => "ambiguous_target",
            Self::StaleReference => "stale_ref",
            Self::InvalidResult(_) => "unsupported_result",
            Self::Configuration(_) => "invalid_input",
            Self::Io(_) => "io_error",
            Self::Cdp(_) | Self::Events(_) | Self::Task(_) => "driver_error",
        }
    }

    pub fn details(&self) -> serde_json::Value {
        match self {
            Self::Action { details, .. } | Self::PartialFailure { details } => details.clone(),
            Self::Cleanup {
                operation: Some(error),
                ..
            } => error.details(),
            Self::NotActionable {
                condition,
                interceptor,
            } => {
                let mut result = serde_json::json!({"condition": condition});
                if let Some(interceptor) = interceptor {
                    result["interceptor"] = serde_json::json!(interceptor);
                }
                result
            }
            Self::Ambiguous(count) => serde_json::json!({"count": count}),
            _ => serde_json::json!({}),
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
        assert_eq!(failure.code(), "not_actionable");
        assert_eq!(failure.details()["condition"], "hit");
        assert_eq!(failure.details()["interceptor"], "overlay");
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
            assert_eq!(error.code(), code);
        }
        let ended = CancellationToken::new();
        let cancelled = CancellationToken::new();
        let operation = Operation::new(&ended, &cancelled, CONTROL_TIMEOUT);
        assert_eq!(
            operation
                .failure(BrowserError::Timeout, "tab", None)
                .details()["action"],
            "not_started"
        );
        operation.begin_input();
        assert_eq!(
            operation
                .failure(BrowserError::Closed, "tab", None)
                .details()["action"],
            "unknown"
        );
        operation.complete_input();
        assert_eq!(
            operation
                .failure(BrowserError::Timeout, "tab", None)
                .details()["action"],
            "completed"
        );
    }
}
