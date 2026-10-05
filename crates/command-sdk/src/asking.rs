//! A stream the runner opens into a service and answers the service's
//! requests on, at both ends (`native-runtime.md` § Conversation numbers,
//! § The artifacts stream). The service writes each request as one standard
//! output record with an id of its own, unique among its requests in flight;
//! the runner sends each answer back as one input chunk when the service
//! pulls. Before its outcome, a request may be answered with how far it has
//! come, as an install's download is. The numbers stream and the artifacts
//! stream are two kinds of it.

use std::{
    collections::{HashMap, VecDeque},
    future::Future,
};

use bytes::Bytes;
use futures_util::{StreamExt, stream::FuturesUnordered};
use serde::{Serialize, de::DeserializeOwned};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::{CommandInput, CommandOutput, Input, Output, ServiceError};
use demi_command_protocol::{Completion, ProtocolError, Record};

/// Requests waiting for the stream; a full queue holds back their callers.
const ASKS: usize = 64;
/// The most requests of one service the answering end has in flight; one
/// beyond is refused at once.
const IN_FLIGHT: usize = 32;
/// Progress reports a caller has not taken yet; one beyond is not delivered.
const PROGRESS: usize = 16;

/// One kind of stream: its request and answer records and what an answer
/// gives back.
pub trait Asked: Send + Sync + 'static {
    type Request: Serialize + DeserializeOwned + Send + 'static;
    type Answer: Serialize + DeserializeOwned + Send + 'static;
    type Reply: Send + 'static;
    /// How far a request has come before its outcome.
    type Progress: Send + 'static;
    /// The stream's name, for the log and for errors.
    const NAME: &'static str;
    /// `request` with the service's id `id`.
    fn with_id(request: Self::Request, id: u64) -> Self::Request;
    fn request_id(request: &Self::Request) -> u64;
    /// Checks a request where it is made and where it is read.
    fn check(request: &Self::Request) -> Result<(), ProtocolError>;
    fn answer(id: u64, result: Result<Self::Reply, String>) -> Self::Answer;
    /// The answer that reports how far request `id` has come.
    fn progress(id: u64, progress: Self::Progress) -> Self::Answer;
    fn answer_id(answer: &Self::Answer) -> u64;
    fn answered(answer: &Self::Answer) -> Result<Answered<Self>, ProtocolError>;
}

/// What one answer says about its request: how far it has come, or its
/// outcome.
pub enum Answered<A: Asked + ?Sized> {
    Progress(A::Progress),
    Outcome(Result<A::Reply, String>),
}

/// A request waiting for its answer.
pub struct Pending<A: Asked> {
    pub request: A::Request,
    pub answer: oneshot::Sender<Result<A::Reply, String>>,
    /// Where the request's progress goes, when its caller follows it.
    pub progress: Option<mpsc::Sender<A::Progress>>,
}

/// Where a service's requests of one kind go. Cloning shares the one stream.
pub struct Asker<A: Asked> {
    asks: mpsc::Sender<Pending<A>>,
}

impl<A: Asked> Clone for Asker<A> {
    fn clone(&self) -> Self {
        Self {
            asks: self.asks.clone(),
        }
    }
}

impl<A: Asked> Asker<A> {
    /// An asker whose requests arrive at the returned receiver, where the
    /// service's stream, or a test, answers them.
    pub fn channel() -> (Self, mpsc::Receiver<Pending<A>>) {
        let (asks, received) = mpsc::channel(ASKS);
        (Self { asks }, received)
    }

    /// The runner's answer to `request`, or why it gave none; a request made
    /// before the runner opened the stream waits for it. `None` when the
    /// stream has ended. The runner's reports of how far the request has
    /// come go to the receiver of [`Asker::progress`] given as `progress`.
    pub async fn ask(
        &self,
        request: A::Request,
        progress: Option<mpsc::Sender<A::Progress>>,
    ) -> Option<Result<A::Reply, String>> {
        let (answer, answered) = oneshot::channel();
        let pending = Pending {
            request,
            answer,
            progress,
        };
        self.asks.send(pending).await.ok()?;
        answered.await.ok()
    }

    /// Where a caller that follows its request's progress has it sent, and
    /// where it reads it. The reports end with the request's answer.
    pub fn progress() -> (mpsc::Sender<A::Progress>, mpsc::Receiver<A::Progress>) {
        mpsc::channel(PROGRESS)
    }
}

/// Serves the stream in the service: writes each request as one record and
/// hands each answer, one input chunk, to the request it answers. It
/// completes at `finish`, as the service shuts down, when the runner ends
/// the input, or when no asker is left; a request still waiting then fails.
pub(crate) async fn relay<A: Asked>(
    mut asks: mpsc::Receiver<Pending<A>>,
    mut input: Input,
    output: Output,
    finish: CancellationToken,
) -> Result<Completion, ServiceError> {
    // Both halves run in this one task, and neither holds the lock across a
    // wait.
    let waiting = std::sync::Mutex::new(HashMap::new());
    let writer = async {
        let mut next = 0_u64;
        while let Some(pending) = asks.recv().await {
            let id = next;
            next += 1;
            let request = A::with_id(pending.request, id);
            let bytes = Bytes::from(serde_json::to_vec(&request)?);
            waiting
                .lock()
                .expect("the waiting requests are never poisoned")
                .insert(id, (pending.answer, pending.progress));
            output.stdout(bytes).await?;
        }
        Ok::<_, ServiceError>(())
    };
    let reader = async {
        while let Some(chunk) = input.next().await? {
            let answer: A::Answer = serde_json::from_slice(&chunk)?;
            let id = A::answer_id(&answer);
            let mut waiting = waiting
                .lock()
                .expect("the waiting requests are never poisoned");
            match A::answered(&answer)? {
                Answered::Progress(progress) => {
                    // A caller that follows no progress, or has not taken
                    // the reports before, misses this one; its answer still
                    // comes.
                    if let Some((_, Some(follower))) = waiting.get(&id) {
                        let _missed = follower.try_send(progress);
                    }
                }
                Answered::Outcome(outcome) => {
                    // A caller that stopped waiting needs no answer.
                    if let Some((answer, _)) = waiting.remove(&id) {
                        let _left = answer.send(outcome);
                    }
                }
            }
        }
        Ok::<_, ServiceError>(())
    };
    tokio::select! {
        written = writer => written?,
        read = reader => read?,
        () = finish.cancelled() => {}
    }
    Ok(Completion {
        exit_code: 0,
        error: None,
    })
}

/// A service's open stream, which the runner answers.
pub struct RequestStream {
    pub(crate) input: CommandInput,
    pub(crate) output: CommandOutput,
}

/// Where the answering end reports how far one request has come.
pub struct Reporter<A: Asked> {
    id: u64,
    reports: mpsc::UnboundedSender<(u64, A::Progress)>,
}

impl<A: Asked> Reporter<A> {
    /// Tells the service how far the request has come; it goes before the
    /// request's outcome.
    pub fn report(&self, progress: A::Progress) {
        // The stream's loop holds the receiver until it returns, and then
        // nothing is answered any more.
        let _ended = self.reports.send((self.id, progress));
    }
}

impl RequestStream {
    /// Answers the service's requests until it ends the stream: each goes
    /// to `answer`, at most 32 at a time, with where it reports how far it
    /// has come, and each answer goes back as one input chunk when the
    /// service pulls.
    pub async fn answer<A: Asked, F>(
        mut self,
        answer: impl Fn(A::Request, Reporter<A>) -> F,
    ) -> Result<(), ServiceError>
    where
        F: Future<Output = Result<A::Reply, String>>,
    {
        let mut asking = FuturesUnordered::new();
        let mut answers = VecDeque::new();
        let mut pulls = 0_usize;
        // Unbounded: each request's answer reports a bounded number of
        // times, such as a download's tenths.
        let (reporting, mut reports) = mpsc::unbounded_channel();
        loop {
            // Each pull lets one answer go.
            while pulls > 0
                && let Some(answered) = answers.pop_front()
            {
                self.input.write(encode::<A>(&answered)?).await?;
                pulls -= 1;
            }
            tokio::select! {
                record = self.output.next() => match record? {
                    Some(Record::Stdout(bytes)) => {
                        let request: A::Request = serde_json::from_slice(&bytes)?;
                        A::check(&request)?;
                        let id = A::request_id(&request);
                        if asking.len() < IN_FLIGHT {
                            let reporter = Reporter {
                                id,
                                reports: reporting.clone(),
                            };
                            let answering = answer(request, reporter);
                            asking.push(async move { A::answer(id, answering.await) });
                        } else {
                            let refused = Err(format!("too many {} requests in flight", A::NAME));
                            answers.push_back(A::answer(id, refused));
                        }
                    }
                    Some(Record::InputPull) => pulls += 1,
                    Some(Record::Stderr(bytes)) => tracing::info!(
                        "{} stream: {}",
                        A::NAME,
                        String::from_utf8_lossy(&bytes).trim_end()
                    ),
                    // A stream's writer refuses media.
                    Some(Record::Medium { .. } | Record::MediumBytes(_)) => {
                        return Err(ProtocolError::Invalid(format!("a medium on the {} stream", A::NAME)).into());
                    }
                    // The service ended the stream, as its shutdown does.
                    Some(Record::Completion(_)) | None => return Ok(()),
                },
                Some((id, progress)) = reports.recv() => answers.push_back(A::progress(id, progress)),
                Some(answered) = asking.next(), if !asking.is_empty() => {
                    // What the request reported before its outcome goes first.
                    while let Ok((id, progress)) = reports.try_recv() {
                        answers.push_back(A::progress(id, progress));
                    }
                    answers.push_back(answered);
                }
            }
        }
    }
}

fn encode<A: Asked>(answer: &A::Answer) -> Result<Bytes, ServiceError> {
    Ok(Bytes::from(serde_json::to_vec(answer)?))
}
