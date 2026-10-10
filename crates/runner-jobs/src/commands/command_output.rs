//! Invocation output with bounded JSON capture when the caller passes `--json`
//! (`commands.md` § Deliver IO and release an invocation), and the media a
//! command returns, which go to its job whatever its stdout is
//! (`commands.md` § Return media).

use std::sync::Arc;

use bytes::Bytes;
use demi_command_declarations::Schema;
use demi_command_protocol::{MAX_MEDIUM_BYTES, MediumFacts, sniff_media_type};
use demi_command_sdk::{Output, OutputSink, ServiceError};

use crate::job_media::JobMedia;

/// The most JSON output a command may produce.
const JSON_BYTES: usize = 1024 * 1024;

/// One command's output. With `--json` its standard output is captured whole,
/// so it reaches the caller only as JSON that matches the leaf's output
/// schema; its standard error passes through. A medium it returns goes to
/// the job, never into its stdout.
pub struct CommandOutput<'a> {
    output: Output,
    json: Option<Capture<'a>>,
    /// The job its media go to; none when its leaf does not declare
    /// `media`, so that a medium fails it.
    media: Option<Arc<JobMedia>>,
    /// Whether it returned a medium.
    returned: bool,
}

/// A `--json` command's standard output so far, and the schema it must match.
struct Capture<'a> {
    schema: &'a Schema,
    bytes: Vec<u8>,
}

impl<'a> CommandOutput<'a> {
    /// `json` is the leaf's output schema when the caller passed `--json`.
    pub fn new(output: Output, json: Option<&'a Schema>, media: Option<Arc<JobMedia>>) -> Self {
        Self {
            output,
            json: json.map(|schema| Capture {
                schema,
                bytes: Vec::new(),
            }),
            media,
            returned: false,
        }
    }

    pub async fn stdout(&mut self, bytes: Bytes) -> Result<(), ServiceError> {
        if let Some(capture) = &mut self.json {
            if capture.bytes.len() + bytes.len() > JSON_BYTES {
                return Err(ServiceError::failed(OutputError::TooLarge));
            }
            capture.bytes.extend_from_slice(&bytes);
            return Ok(());
        }
        if bytes.is_empty() {
            return Ok(());
        }
        self.output.stdout(bytes).await
    }

    pub async fn stderr(&self, bytes: Bytes) -> Result<(), ServiceError> {
        self.output.stderr(bytes).await
    }

    /// Where standard error goes, for work that writes it beside standard output.
    pub fn errors(&self) -> Output {
        self.output.clone()
    }

    /// A medium the command returned, which `facts` describe: checked, then
    /// handed to the job.
    pub async fn medium(&mut self, facts: MediumFacts, bytes: Bytes) -> Result<(), ServiceError> {
        let Some(job) = &self.media else {
            return Err(ServiceError::failed(MediaFailure::Undeclared));
        };
        if bytes.len() as u64 > MAX_MEDIUM_BYTES {
            return Err(ServiceError::failed(MediaFailure::TooLarge(bytes.len())));
        }
        if sniff_media_type(&bytes) != Some(facts.media_type.as_str()) {
            return Err(ServiceError::failed(MediaFailure::NotMedia(facts.media_type)));
        }
        self.returned = true;
        job.keep(facts, bytes).await.map_err(ServiceError::failed)
    }

    /// Releases captured output once the command has succeeded; a command
    /// that failed releases none. A command that returned media may print
    /// no value.
    pub async fn finish(mut self, exit_code: u8) -> Result<(), ServiceError> {
        let Some(capture) = self.json.take() else {
            return Ok(());
        };
        if exit_code != 0 || (self.returned && capture.bytes.is_empty()) {
            return Ok(());
        }
        let value: serde_json::Value = serde_json::from_slice(&capture.bytes)
            .map_err(|error| ServiceError::failed(OutputError::NotJson(error)))?;
        capture
            .schema
            .check(&value)
            .map_err(|error| ServiceError::failed(OutputError::Invalid(error)))?;
        self.output.stdout(capture.bytes.into()).await
    }
}

impl OutputSink for CommandOutput<'_> {
    type Error = ServiceError;

    async fn stdout(&mut self, bytes: Bytes) -> Result<(), ServiceError> {
        CommandOutput::stdout(self, bytes).await
    }

    async fn stderr(&mut self, bytes: Bytes) -> Result<(), ServiceError> {
        CommandOutput::stderr(self, bytes).await
    }

    async fn medium(&mut self, facts: MediumFacts, bytes: Bytes) -> Result<(), ServiceError> {
        CommandOutput::medium(self, facts, bytes).await
    }
}

/// Why a command's `--json` output does not reach its caller.
#[derive(Debug, thiserror::Error)]
enum OutputError {
    #[error("--json output exceeds 1 MiB")]
    TooLarge,
    #[error("--json output is not JSON: {0}")]
    NotJson(serde_json::Error),
    #[error("--json output does not match its schema: {0}")]
    Invalid(String),
}

/// Why a command's medium failed it (`commands.md` § Return media).
#[derive(Debug, thiserror::Error)]
enum MediaFailure {
    #[error("returned a medium, but its declaration does not say it returns media")]
    Undeclared,
    #[error("returned a medium of {0} bytes; a medium is at most 16 MiB")]
    TooLarge(usize),
    #[error("returned a medium whose bytes are no {0}")]
    NotMedia(String),
}
