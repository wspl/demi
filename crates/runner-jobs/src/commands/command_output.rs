//! Invocation output with bounded JSON capture when the caller passes `--json`
//! (`commands.md` § Deliver IO and release an invocation).

use bytes::Bytes;
use demi_command_service::{Output, OutputSink, ServiceError};
use demi_command_tree::Schema;

/// The most JSON output a command may produce.
const JSON_BYTES: usize = 1024 * 1024;

/// One command's output. With `--json` its standard output is captured whole,
/// so it reaches the caller only as JSON that matches the leaf's output
/// schema; its standard error passes through.
pub struct CommandOutput<'a> {
    output: Output,
    json: Option<Capture<'a>>,
}

/// A `--json` command's standard output so far, and the schema it must match.
struct Capture<'a> {
    schema: &'a Schema,
    bytes: Vec<u8>,
}

impl<'a> CommandOutput<'a> {
    /// `json` is the leaf's output schema when the caller passed `--json`.
    pub fn new(output: Output, json: Option<&'a Schema>) -> Self {
        Self {
            output,
            json: json.map(|schema| Capture {
                schema,
                bytes: Vec::new(),
            }),
        }
    }

    pub async fn stdout(&mut self, bytes: Bytes) -> Result<(), ServiceError> {
        if let Some(capture) = &mut self.json {
            if capture.bytes.len() + bytes.len() > JSON_BYTES {
                return Err(ServiceError::failed(OutputError::TooLarge));
            }
            capture.bytes.extend_from_slice(&bytes);
            Ok(())
        } else {
            self.output.stdout(bytes).await
        }
    }

    pub async fn stderr(&self, bytes: Bytes) -> Result<(), ServiceError> {
        self.output.stderr(bytes).await
    }

    /// Where standard error goes, for work that writes it beside standard output.
    pub fn errors(&self) -> Output {
        self.output.clone()
    }

    /// Releases captured output once the command has succeeded; a command
    /// that failed releases none.
    pub async fn finish(self, exit_code: u8) -> Result<(), ServiceError> {
        if exit_code != 0 {
            return Ok(());
        }
        if let Some(capture) = self.json {
            let value: serde_json::Value = serde_json::from_slice(&capture.bytes)
                .map_err(|error| ServiceError::failed(OutputError::NotJson(error)))?;
            capture
                .schema
                .check(&value)
                .map_err(|error| ServiceError::failed(OutputError::Invalid(error)))?;
            self.output.stdout(capture.bytes.into()).await?;
        }
        Ok(())
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
