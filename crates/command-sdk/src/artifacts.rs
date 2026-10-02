//! The artifacts stream (`native-runtime.md` § The artifacts stream): the
//! handler asks the runner to install an artifact for one of its
//! invocations, or which artifacts of a line the Host has, through its
//! [`Artifacts`], and uses only the paths the runner answers.

use std::path::PathBuf;

use tokio::sync::mpsc;

use crate::ServiceError;
use crate::asking::{Asked, Asker, Pending};
use demi_command_protocol::{
    ArtifactAnswer, ArtifactAsk, ArtifactInstall, ArtifactReply, ArtifactRequest,
    ArtifactsInstalled, InstalledArtifact, ProtocolError,
};

/// The artifacts stream's kind of request.
pub struct ArtifactsAsk;

impl Asked for ArtifactsAsk {
    type Request = ArtifactRequest;
    type Answer = ArtifactAnswer;
    type Reply = ArtifactReply;
    const NAME: &'static str = "artifacts";

    fn with_id(request: ArtifactRequest, id: u64) -> ArtifactRequest {
        ArtifactRequest { id, ..request }
    }

    fn request_id(request: &ArtifactRequest) -> u64 {
        request.id
    }

    fn check(request: &ArtifactRequest) -> Result<(), ProtocolError> {
        request.ask().map(drop)
    }

    fn answer(id: u64, result: Result<ArtifactReply, String>) -> ArtifactAnswer {
        ArtifactAnswer::new(id, result)
    }

    fn answer_id(answer: &ArtifactAnswer) -> u64 {
        answer.id
    }

    fn outcome(answer: &ArtifactAnswer) -> Result<Result<ArtifactReply, String>, ProtocolError> {
        answer.outcome()
    }
}

/// A request waiting for its answer.
pub type ArtifactPending = Pending<ArtifactsAsk>;

/// Asks the runner for artifacts. Cloning shares the one source.
#[derive(Clone)]
pub struct Artifacts {
    asker: Asker<ArtifactsAsk>,
}

impl Artifacts {
    /// A source whose requests arrive at the returned receiver, where the
    /// service's artifacts stream, or a test, answers them.
    pub fn channel() -> (Self, mpsc::Receiver<ArtifactPending>) {
        let (asker, requests) = Asker::channel();
        (Self { asker }, requests)
    }

    /// Installs `install` for the invocation it names, and answers the path
    /// of its file or of its archive's entry.
    pub async fn install(&self, install: ArtifactInstall) -> Result<PathBuf, ServiceError> {
        match self.ask(ArtifactAsk::Install(install)).await? {
            ArtifactReply::Path(path) => Ok(PathBuf::from(path)),
            ArtifactReply::Installed(_) => Err(unexpected()),
        }
    }

    /// The artifacts of the line `name` the Host has, the newest install
    /// first.
    pub async fn installed(&self, name: &str) -> Result<Vec<InstalledArtifact>, ServiceError> {
        let question = ArtifactsInstalled {
            name: name.to_owned(),
        };
        match self.ask(ArtifactAsk::Installed(question)).await? {
            ArtifactReply::Installed(installed) => Ok(installed),
            ArtifactReply::Path(_) => Err(unexpected()),
        }
    }

    async fn ask(&self, ask: ArtifactAsk) -> Result<ArtifactReply, ServiceError> {
        // Checked here, where the caller learns why, rather than where the
        // runner reads it and breaks the stream.
        let request = ArtifactRequest::new(0, ask);
        request.ask()?;
        self.asker
            .ask(request)
            .await
            .ok_or_else(|| ServiceError::Artifacts("the artifacts stream has ended".into()))?
            .map_err(ServiceError::Artifacts)
    }
}

fn unexpected() -> ServiceError {
    ServiceError::Artifacts("the runner answered another kind of request".into())
}
