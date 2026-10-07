//! The web preview's stream, `browser.preview` (`preview.md` § The stream):
//! one engine for every conversation of the Host, opened with the first
//! stream, whose cookie jar this program keeps in the data directory its
//! runner instance names (`native-runtime.md` § Invoke and retire a
//! service).

use std::path::PathBuf;
use std::sync::Arc;

use bytes::Bytes;
use demi_command_package_browser_preview::{Engine, StreamError};
use demi_command_protocol::{CommandError, Completion};
use demi_command_sdk::{InvocationContext, ServiceError};
use tokio::sync::OnceCell;
use tokio_util::sync::CancellationToken;
use tokio_util::task::{AbortOnDropHandle, TaskTracker};

/// The jar's file in the data directory.
const JAR_FILE: &str = "preview-cookies.json";

/// The preview engine of the program and its open streams.
pub(crate) struct Previews {
    /// Where the engine keeps its jar; none when the runner named no data
    /// directory.
    directory: Option<PathBuf>,
    engine: OnceCell<Arc<Engine>>,
    streams: TaskTracker,
    stopping: CancellationToken,
}

impl Previews {
    pub fn new(directory: Option<PathBuf>) -> Self {
        Self {
            directory,
            engine: OnceCell::new(),
            streams: TaskTracker::new(),
            stopping: CancellationToken::new(),
        }
    }

    async fn engine(&self) -> Result<Arc<Engine>, ServiceError> {
        self.engine
            .get_or_try_init(|| async {
                let directory = self
                    .directory
                    .as_ref()
                    .ok_or_else(|| ServiceError::failed(std::io::Error::other("the runner named no data directory for the preview's cookie jar")))?;
                Engine::open(directory.join(JAR_FILE)).map_err(ServiceError::failed)
            })
            .await
            .cloned()
    }

    /// Serves one stream until the page ends it, the invocation is
    /// cancelled, or the program stops.
    pub async fn serve(&self, context: InvocationContext) -> Result<Completion, ServiceError> {
        let engine = self.engine().await?;
        let _open = self.streams.token();
        let InvocationContext {
            input,
            output,
            cancellation,
            ..
        } = context;
        let stop = self.stopping.child_token();
        let _cancelled = AbortOnDropHandle::new(tokio::spawn({
            let (cancellation, stop) = (cancellation.clone(), stop.clone());
            async move {
                cancellation.cancelled().await;
                stop.cancel();
            }
        }));
        let input = Box::pin(futures_util::stream::unfold(input, |mut input| async move {
            match input.next().await {
                Ok(Some(bytes)) => Some((Ok(bytes), input)),
                Ok(None) => None,
                Err(error) => Some((Err(std::io::Error::other(error)), input)),
            }
        }));
        let output = Box::pin(futures_util::sink::unfold(output, |output, bytes: Bytes| async move {
            output.stdout(bytes).await.map_err(std::io::Error::other)?;
            Ok::<_, std::io::Error>(output)
        }));
        let result = demi_command_package_browser_preview::serve(engine, input, output, stop).await;
        if cancellation.is_cancelled() {
            return Err(ServiceError::Cancelled);
        }
        match result {
            Ok(()) => Ok(Completion {
                exit_code: 0,
                error: None,
            }),
            Err(StreamError::Protocol(message)) => Ok(Completion {
                exit_code: 2,
                error: Some(CommandError {
                    code: "invalid_input".into(),
                    message,
                }),
            }),
            Err(error) => Err(ServiceError::failed(error)),
        }
    }

    /// Ends every stream, then writes the jar's latest changes.
    pub async fn close(&self) -> Result<(), ServiceError> {
        self.stopping.cancel();
        self.streams.close();
        self.streams.wait().await;
        match self.engine.get() {
            Some(engine) => engine.close().await.map_err(ServiceError::failed),
            None => Ok(()),
        }
    }
}
