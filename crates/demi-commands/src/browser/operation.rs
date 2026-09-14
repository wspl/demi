use std::{future::Future, time::Duration};

use chromiumoxide::error::CdpError;
use thiserror::Error;
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

pub(super) const CONTROL_TIMEOUT: Duration = Duration::from_secs(5);

pub type Result<T> = std::result::Result<T, BrowserError>;

#[derive(Debug, Error)]
pub enum BrowserError {
    #[error("browser environment is closed")]
    Closed,
    #[error("{0}")]
    Connection(String),
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
    Cdp(#[from] CdpError),
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
}

impl<'a> Operation<'a> {
    pub fn new(
        ended: &'a CancellationToken,
        cancelled: &'a CancellationToken,
        timeout: Duration,
    ) -> Self {
        Self {
            ended,
            cancelled,
            deadline: Instant::now() + timeout,
        }
    }

    pub async fn run<T>(&self, future: impl Future<Output = Result<T>>) -> Result<T> {
        tokio::select! {
            biased;
            _ = self.ended.cancelled() => Err(BrowserError::Closed),
            _ = self.cancelled.cancelled() => Err(BrowserError::Cancelled),
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
