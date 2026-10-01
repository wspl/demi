//! The numbers stream (`native-runtime.md` § Conversation numbers), at both
//! ends. In the service, the handler draws numbers of a conversation's
//! sequence from its [`Numbers`], and the stream carries each draw to the
//! runner; the runner answers the stream with the numbers the backend
//! reserved ([`NumbersStream::answer`]).

use std::{
    collections::{HashMap, VecDeque},
    future::Future,
};

use bytes::Bytes;
use futures_util::{StreamExt, stream::FuturesUnordered};
use tokio::sync::{mpsc, oneshot};
use tokio_util::sync::CancellationToken;

use crate::{CommandInput, CommandOutput, Input, Output, ServiceError};
use demi_command_protocol::{Completion, NumbersAnswer, NumbersRequest, Record, ServiceSequence};

/// Draws waiting for the numbers stream; a full queue holds back their
/// callers.
const DRAWS: usize = 64;
/// The most requests of one service the answering end has in flight; one
/// beyond is refused at once.
const IN_FLIGHT: usize = 32;

/// Draws numbers from the conversations' sequences. Cloning shares the one
/// source.
#[derive(Clone)]
pub struct Numbers {
    draws: mpsc::Sender<Draw>,
}

/// A draw waiting for its answer.
pub struct Draw {
    pub conversation: String,
    pub sequence: ServiceSequence,
    pub count: u32,
    pub answer: oneshot::Sender<Result<u64, String>>,
}

impl Numbers {
    /// A source whose draws arrive at the returned receiver, where the
    /// service's numbers stream, or a test, answers them.
    pub fn channel() -> (Self, mpsc::Receiver<Draw>) {
        let (draws, received) = mpsc::channel(DRAWS);
        (Self { draws }, received)
    }

    /// The first of `count` consecutive numbers of `conversation`'s
    /// `sequence`, which now belong to the caller. A draw made before the
    /// runner opened the numbers stream waits for it.
    pub async fn draw(
        &self,
        conversation: &str,
        sequence: ServiceSequence,
        count: u32,
    ) -> Result<u64, ServiceError> {
        // Checked here, where the caller learns why, rather than where the
        // runner reads it and breaks the stream.
        let request = NumbersRequest {
            id: 0,
            conversation: conversation.to_owned(),
            sequence,
            count,
        };
        request.validate()?;
        let (answer, answered) = oneshot::channel();
        let ended = || ServiceError::Numbers("the numbers stream has ended".into());
        self.draws
            .send(Draw {
                conversation: request.conversation,
                sequence,
                count,
                answer,
            })
            .await
            .map_err(|_| ended())?;
        answered
            .await
            .map_err(|_| ended())?
            .map_err(ServiceError::Numbers)
    }
}

/// Serves the numbers stream: writes each draw as one request record and
/// hands each answer, one input chunk, to the draw it answers. It completes
/// at `finish`, as the service shuts down, when the runner ends the input,
/// or when no source is left to draw; a draw still waiting then fails.
pub(crate) async fn relay(
    mut draws: mpsc::Receiver<Draw>,
    mut input: Input,
    output: Output,
    finish: CancellationToken,
) -> Result<Completion, ServiceError> {
    // Both halves run in this one task, and neither holds the lock across a
    // wait.
    let waiting = std::sync::Mutex::new(HashMap::new());
    let writer = async {
        let mut next = 0_u64;
        while let Some(draw) = draws.recv().await {
            let id = next;
            next += 1;
            let request = NumbersRequest {
                id,
                conversation: draw.conversation,
                sequence: draw.sequence,
                count: draw.count,
            };
            let bytes = Bytes::from(serde_json::to_vec(&request)?);
            waiting
                .lock()
                .expect("the waiting draws are never poisoned")
                .insert(id, draw.answer);
            output.stdout(bytes).await?;
        }
        Ok::<_, ServiceError>(())
    };
    let reader = async {
        while let Some(chunk) = input.next().await? {
            let answer: NumbersAnswer = serde_json::from_slice(&chunk)?;
            let outcome = answer.outcome()?;
            let waiting = waiting
                .lock()
                .expect("the waiting draws are never poisoned")
                .remove(&answer.id);
            // A caller that stopped waiting needs no answer.
            if let Some(waiting) = waiting {
                let _left = waiting.send(outcome);
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

/// A service's open numbers stream, which the caller answers.
pub struct NumbersStream {
    pub(crate) input: CommandInput,
    pub(crate) output: CommandOutput,
}

impl NumbersStream {
    /// Answers the service's requests until it ends the stream: each goes
    /// to `reserve`, at most 32 at a time, and each answer goes back as one
    /// input chunk when the service pulls.
    pub async fn answer<F>(
        mut self,
        reserve: impl Fn(NumbersRequest) -> F,
    ) -> Result<(), ServiceError>
    where
        F: Future<Output = Result<u64, String>>,
    {
        let mut asking = FuturesUnordered::new();
        let mut answers = VecDeque::new();
        let mut pulls = 0_usize;
        loop {
            // Each pull lets one answer go.
            while pulls > 0
                && let Some(answer) = answers.pop_front()
            {
                self.input.write(encode(&answer)?).await?;
                pulls -= 1;
            }
            tokio::select! {
                record = self.output.next() => match record? {
                    Some(Record::Stdout(bytes)) => {
                        let request: NumbersRequest = serde_json::from_slice(&bytes)?;
                        request.validate()?;
                        let id = request.id;
                        if asking.len() < IN_FLIGHT {
                            let reserved = reserve(request);
                            asking.push(async move { NumbersAnswer::new(id, reserved.await) });
                        } else {
                            let refused = Err("too many number requests in flight".to_owned());
                            answers.push_back(NumbersAnswer::new(id, refused));
                        }
                    }
                    Some(Record::InputPull) => pulls += 1,
                    Some(Record::Stderr(bytes)) => tracing::info!(
                        "numbers stream: {}",
                        String::from_utf8_lossy(&bytes).trim_end()
                    ),
                    // The service ended the stream, as its shutdown does.
                    Some(Record::Completion(_)) | None => return Ok(()),
                },
                Some(answer) = asking.next(), if !asking.is_empty() => answers.push_back(answer),
            }
        }
    }
}

fn encode(answer: &NumbersAnswer) -> Result<Bytes, ServiceError> {
    Ok(Bytes::from(serde_json::to_vec(answer)?))
}
