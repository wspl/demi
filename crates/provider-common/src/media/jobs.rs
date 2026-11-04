//! A media job a vendor keeps by its id: asked once, then read by the id
//! until it ends, so whoever keeps the id reads it again after a restart.
//! Every video is a job; an image or a sound is one where its vendor makes
//! it one ([`crate::images::ImageMaker::makes_jobs`],
//! [`crate::audio::AudioMaker::makes_jobs`]).

use std::time::Duration;

use demi_shared_types::VendorTrace;
use futures_util::future::BoxFuture;

use super::{MediaAnswer, MediaFailure, unreadable};
use crate::media_catalog::Catalog;

/// How often [`waited`] reads a job, as a vendor's own guide to its jobs
/// reads them: often enough that an image a few seconds long is not kept
/// waiting, seldom enough that no vendor limits the reads.
pub const READS_EVERY: Duration = Duration::from_secs(2);

/// A job's state as one read of it says; `T` is what a finished job made.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub enum JobState<T> {
    /// Waiting for the vendor to start it.
    Queued,
    /// Being made, with how far along when the vendor says.
    Running { progress: Option<u8> },
    Done(T),
    /// The vendor failed the job; `reason` is Demi's, for a failure it read
    /// out of an answer, such as an image withheld, and `trace` what the
    /// vendor said. Neither crosses a relay.
    Failed {
        #[serde(skip)]
        reason: Option<String>,
        #[serde(skip)]
        trace: VendorTrace,
    },
    /// The job expired at the vendor before it ran.
    Expired,
    /// The server that served the job through a channel no longer knows it:
    /// it restarted and lost the job, or kept it past the time it is read.
    Lost,
}

impl<T> JobState<T> {
    /// The state with what a finished job made changed by `change`.
    pub fn map<U>(self, change: impl FnOnce(T) -> U) -> JobState<U> {
        match self {
            Self::Queued => JobState::Queued,
            Self::Running { progress } => JobState::Running { progress },
            Self::Done(made) => JobState::Done(change(made)),
            Self::Failed { reason, trace } => JobState::Failed { reason, trace },
            Self::Expired => JobState::Expired,
            Self::Lost => JobState::Lost,
        }
    }
}

/// A job a vendor took: its id at the vendor, and the model it makes as
/// `catalog` states it, by which a vendor whose models take different
/// requests, as MiniMax's do, reads its state.
#[derive(Debug, Clone, Copy)]
pub struct JobRef<'a> {
    pub id: &'a str,
    pub model: &'a str,
    pub catalog: &'a Catalog,
}

/// A maker's jobs: requests of `R` it asks for as jobs the vendor keeps by
/// their ids, each of which makes a `T`.
pub trait Jobs<R, T>: Send + Sync {
    /// Asks for what `request` describes and answers the job's id;
    /// `catalog` says what the model takes.
    fn ask<'a>(&'a self, http: &'a crate::ProviderHttp, catalog: &'a Catalog, request: R) -> BoxFuture<'a, MediaAnswer<String>>;

    /// Reads the state of the job `job` once, a finished job's file within
    /// it.
    fn state<'a>(&'a self, http: &'a crate::ProviderHttp, job: JobRef<'a>) -> BoxFuture<'a, MediaAnswer<JobState<T>>>;
}

/// What `jobs` makes of `request` in one request: [`waited`] for its job.
pub async fn made<R, T>(jobs: &dyn Jobs<R, T>, http: &crate::ProviderHttp, catalog: &Catalog, request: R, model: &str) -> MediaAnswer<T> {
    let asked = jobs.ask(http, catalog, request).await;
    waited(asked, |id| {
        Box::pin(async move { jobs.state(http, JobRef { id: &id, model, catalog }).await })
    })
    .await
}

/// What a job a maker answers in one request made: its ask, then a read
/// every [`READS_EVERY`] until it ends. A job that ends without what it
/// was asked for fails as a refusal, in the vendor's words.
pub async fn waited<'a, T>(
    asked: MediaAnswer<String>,
    mut read: impl FnMut(String) -> BoxFuture<'a, MediaAnswer<JobState<T>>>,
) -> MediaAnswer<T> {
    let mut observed = asked.observed;
    let id = match asked.result {
        Ok(id) => id,
        Err(failure) => return MediaAnswer { result: Err(failure), observed },
    };
    loop {
        let answer = read(id.clone()).await;
        observed.extend(answer.observed);
        let result = match answer.result {
            Ok(JobState::Queued | JobState::Running { .. }) => {
                tokio::time::sleep(READS_EVERY).await;
                continue;
            }
            Ok(JobState::Done(made)) => Ok(made),
            Ok(JobState::Failed { reason, trace }) => Err(MediaFailure::Refused { reason, code: None, trace }),
            Ok(JobState::Expired) => Err(unreadable("its job expired before it ran".to_owned())),
            Ok(JobState::Lost) => Err(unreadable("its job is no longer known".to_owned())),
            Err(failure) => Err(failure),
        };
        return MediaAnswer { result, observed };
    }
}
