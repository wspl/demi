//! Invocation output with bounded JSON capture when a declaration requests it.

use bytes::Bytes;
use demi_command_service::{Output, ServiceError};
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct CommandOutput {
    output: Output,
    json: Option<Arc<Mutex<Vec<u8>>>>,
}

impl CommandOutput {
    pub fn new(output: Output, json: bool) -> Self {
        Self {
            output,
            json: json.then(|| Arc::new(Mutex::new(Vec::new()))),
        }
    }
    pub async fn stdout(&self, bytes: Bytes) -> Result<(), ServiceError> {
        if let Some(capture) = &self.json {
            let mut buffer = capture.lock().await;
            if buffer.len() + bytes.len() > 1024 * 1024 {
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
    pub async fn finish(
        &self,
        exit_code: u8,
        schema: Option<&BTreeMap<String, serde_json::Value>>,
    ) -> Result<(), ServiceError> {
        if exit_code != 0 {
            return Ok(());
        }
        if let Some(capture) = &self.json {
            let bytes = std::mem::take(&mut *capture.lock().await);
            let value: serde_json::Value = serde_json::from_slice(&bytes)?;
            let schema =
                schema.ok_or_else(|| ServiceError::Handler("missing JSON output schema".into()))?;
            let validator = jsonschema::validator_for(&serde_json::to_value(schema)?)
                .map_err(|error| ServiceError::Handler(error.to_string()))?;
            validator.validate(&value).map_err(|error| {
                ServiceError::Handler(format!("JSON output failed validation: {error}"))
            })?;
            self.output.stdout(bytes.into()).await?;
        }
        Ok(())
    }
}
