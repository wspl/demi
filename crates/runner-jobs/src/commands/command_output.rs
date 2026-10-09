//! Invocation output with bounded JSON capture when the caller passes `--json`
//! (`commands.md` § Deliver IO and release an invocation), and the media a
//! command returns, routed by where the calling process's stdout goes
//! (`commands.md` § Return media).

use std::sync::Arc;

use bytes::Bytes;
use demi_command_declarations::Schema;
use demi_command_protocol::{MAX_MEDIUM_BYTES, StdoutTarget, sniff_media_type};
use demi_command_sdk::{Output, OutputSink, ServiceError};

use crate::job_media::{HeldMedium, JobMedia, NOT_KEPT_LINE, medium_line};

/// The most JSON output a command may produce.
const JSON_BYTES: usize = 1024 * 1024;

/// One command's output. With `--json` its standard output is captured whole,
/// so it reaches the caller only as JSON that matches the leaf's output
/// schema; its standard error passes through. A medium it returns goes to
/// the job, its line into stdout, when its stdout is the job's output, and
/// otherwise is held to become its whole stdout once it succeeded.
pub struct CommandOutput<'a> {
    output: Output,
    json: Option<Capture<'a>>,
    /// Where its media go; none when its leaf does not declare `media`,
    /// so that a medium fails it.
    media: Option<Media>,
    /// Whether stdout bytes reached the caller.
    wrote: bool,
    /// Whether the caller's stdout is at the start of a line.
    line_start: bool,
}

/// A `--json` command's standard output so far, and the schema it must match.
struct Capture<'a> {
    schema: &'a Schema,
    bytes: Vec<u8>,
    /// The lines of the media it returned, which follow its value.
    lines: Vec<String>,
}

/// Where the media a command returns go.
pub enum Media {
    /// Its stdout is the job's output: the job keeps them.
    Job(Arc<JobMedia>),
    /// Its stdout goes elsewhere: one medium is held, to become its stdout.
    Elsewhere {
        job: Arc<JobMedia>,
        held: Option<HeldMedium>,
    },
}

impl Media {
    /// Where the media of a command whose stdout goes to `stdout` go, in a
    /// job that keeps media in `job`; none when its leaf does not
    /// `declare` them.
    pub fn new(declared: bool, stdout: StdoutTarget, job: Arc<JobMedia>) -> Option<Self> {
        declared.then_some(match stdout {
            StdoutTarget::Job => Self::Job(job),
            StdoutTarget::Elsewhere => Self::Elsewhere { job, held: None },
        })
    }

    fn holds(&self) -> bool {
        matches!(self, Self::Elsewhere { held: Some(_), .. })
    }
}

impl<'a> CommandOutput<'a> {
    /// `json` is the leaf's output schema when the caller passed `--json`.
    pub fn new(output: Output, json: Option<&'a Schema>, media: Option<Media>) -> Self {
        Self {
            output,
            json: json.map(|schema| Capture {
                schema,
                bytes: Vec::new(),
                lines: Vec::new(),
            }),
            media,
            wrote: false,
            line_start: true,
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
        if self.media.as_ref().is_some_and(Media::holds) {
            return Err(self.beside_stdout());
        }
        self.pass(bytes).await
    }

    pub async fn stderr(&self, bytes: Bytes) -> Result<(), ServiceError> {
        self.output.stderr(bytes).await
    }

    /// Where standard error goes, for work that writes it beside standard output.
    pub fn errors(&self) -> Output {
        self.output.clone()
    }

    /// A medium the command returned: checked, then kept for the job with
    /// its line in stdout, or held.
    pub async fn medium(&mut self, bytes: Bytes) -> Result<(), ServiceError> {
        let Some(media) = &self.media else {
            return Err(self.failure(MediaFailure::Undeclared));
        };
        if bytes.len() as u64 > MAX_MEDIUM_BYTES {
            return Err(self.failure(MediaFailure::TooLarge(bytes.len())));
        }
        let Some(media_type) = sniff_media_type(&bytes) else {
            return Err(self.failure(MediaFailure::NotMedia));
        };
        let job = match media {
            Media::Job(job) => job.clone(),
            Media::Elsewhere { held: Some(_), .. } => {
                return Err(self.failure(MediaFailure::Second));
            }
            Media::Elsewhere { .. } if self.wrote => return Err(self.beside_stdout()),
            Media::Elsewhere { job, held: None } => {
                let held = job.hold(bytes).await.map_err(ServiceError::failed)?;
                if let Some(Media::Elsewhere { held: slot, .. }) = &mut self.media {
                    *slot = Some(held);
                }
                return Ok(());
            }
        };
        let size = bytes.len() as u64;
        let line = match job
            .keep(media_type, bytes)
            .await
            .map_err(ServiceError::failed)?
        {
            Some(number) => medium_line(number, media_type, size),
            None => NOT_KEPT_LINE.to_owned(),
        };
        match &mut self.json {
            Some(capture) => {
                capture.lines.push(line);
                Ok(())
            }
            None => self.line(line).await,
        }
    }

    /// Releases captured output and a held medium once the command has
    /// succeeded; a command that failed releases neither. The lines of the
    /// media the job keeps follow a `--json` value, or stand alone when
    /// there is none: a command that returned media may print no value.
    pub async fn finish(mut self, exit_code: u8) -> Result<(), ServiceError> {
        let holds = self.media.as_ref().is_some_and(Media::holds);
        if let Some(capture) = self.json.take() {
            let returned = holds || !capture.lines.is_empty();
            if exit_code == 0 && holds && !capture.bytes.is_empty() {
                return Err(self.beside_stdout());
            }
            if exit_code == 0 && !(returned && capture.bytes.is_empty()) {
                let value: serde_json::Value = serde_json::from_slice(&capture.bytes)
                    .map_err(|error| ServiceError::failed(OutputError::NotJson(error)))?;
                capture
                    .schema
                    .check(&value)
                    .map_err(|error| ServiceError::failed(OutputError::Invalid(error)))?;
                self.pass(capture.bytes.into()).await?;
            }
            for line in capture.lines {
                self.line(line).await?;
            }
        }
        if exit_code != 0 {
            return Ok(());
        }
        if let Some(Media::Elsewhere {
            held: Some(held), ..
        }) = self.media.take()
        {
            let bytes = held.take().await.map_err(ServiceError::failed)?;
            self.output.stdout(bytes).await?;
        }
        Ok(())
    }

    /// Passes stdout bytes to the caller.
    async fn pass(&mut self, bytes: Bytes) -> Result<(), ServiceError> {
        if let Some(last) = bytes.last() {
            self.wrote = true;
            self.line_start = *last == b'\n';
        }
        self.output.stdout(bytes).await
    }

    /// Writes a medium's line into stdout, on a line of its own.
    async fn line(&mut self, line: String) -> Result<(), ServiceError> {
        let separator = if self.line_start { "" } else { "\n" };
        self.pass(Bytes::from(format!("{separator}{line}\n"))).await
    }

    fn beside_stdout(&self) -> ServiceError {
        self.failure(MediaFailure::BesideStdout)
    }

    fn failure(&self, failure: MediaFailure) -> ServiceError {
        ServiceError::failed(failure)
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

    async fn medium(&mut self, bytes: Bytes) -> Result<(), ServiceError> {
        CommandOutput::medium(self, bytes).await
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
    #[error("returned a medium that is no image or video a model reads")]
    NotMedia,
    #[error(
        "returns 2 media, but its stdout is not the job's output and carries only one; run it once per medium, or let its stdout reach the job's output"
    )]
    Second,
    #[error("a medium must be all of its stdout when its stdout is not the job's output")]
    BesideStdout,
}

