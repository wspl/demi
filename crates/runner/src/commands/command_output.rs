//! Invocation output with bounded JSON capture when a declaration requests it.

use bytes::Bytes;
use demi_command_service::{Output, OutputSink, ServiceError};
use demi_command_tree::Schema;

/// The most JSON output a command may produce.
const JSON_BYTES: usize = 1024 * 1024;

/// One command's output. Its standard output is captured whole when the
/// declaration asks for JSON, so it can be checked before the caller sees
/// it; its standard error passes through.
pub struct CommandOutput {
    output: Output,
    json: Option<Vec<u8>>,
}

impl CommandOutput {
    pub fn new(output: Output, json: bool) -> Self {
        Self {
            output,
            json: json.then(Vec::new),
        }
    }

    pub async fn stdout(&mut self, bytes: Bytes) -> Result<(), ServiceError> {
        if let Some(buffer) = &mut self.json {
            if buffer.len() + bytes.len() > JSON_BYTES {
                return Err(ServiceError::Handler(
                    "JSON command output exceeds 1 MiB".into(),
                ));
            }
            buffer.extend_from_slice(&bytes);
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

    pub async fn finish(self, exit_code: u8, schema: Option<&Schema>) -> Result<(), ServiceError> {
        if exit_code != 0 {
            return Ok(());
        }
        if let Some(bytes) = self.json {
            let value: serde_json::Value = serde_json::from_slice(&bytes)?;
            let schema =
                schema.ok_or_else(|| ServiceError::Handler("missing JSON output schema".into()))?;
            schema.check(&value).map_err(|error| {
                ServiceError::Handler(format!("JSON output failed validation: {error}"))
            })?;
            self.output.stdout(bytes.into()).await?;
        }
        Ok(())
    }
}

impl OutputSink for CommandOutput {
    type Error = ServiceError;

    async fn stdout(&mut self, bytes: Bytes) -> Result<(), ServiceError> {
        CommandOutput::stdout(self, bytes).await
    }

    async fn stderr(&mut self, bytes: Bytes) -> Result<(), ServiceError> {
        CommandOutput::stderr(self, bytes).await
    }
}
