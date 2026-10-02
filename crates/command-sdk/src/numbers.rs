//! The numbers stream (`native-runtime.md` § Conversation numbers): the
//! handler draws numbers of a conversation's sequence from its [`Numbers`],
//! and the runner answers each draw with the numbers the backend reserved.

use tokio::sync::mpsc;

use crate::ServiceError;
use crate::asking::{Asked, Asker, Pending};
use demi_command_protocol::{NumbersAnswer, NumbersRequest, ProtocolError, ServiceSequence};

/// The numbers stream's kind of request.
pub struct NumbersAsk;

impl Asked for NumbersAsk {
    type Request = NumbersRequest;
    type Answer = NumbersAnswer;
    type Reply = u64;
    const NAME: &'static str = "numbers";

    fn with_id(request: NumbersRequest, id: u64) -> NumbersRequest {
        NumbersRequest { id, ..request }
    }

    fn request_id(request: &NumbersRequest) -> u64 {
        request.id
    }

    fn check(request: &NumbersRequest) -> Result<(), ProtocolError> {
        request.validate()
    }

    fn answer(id: u64, result: Result<u64, String>) -> NumbersAnswer {
        NumbersAnswer::new(id, result)
    }

    fn answer_id(answer: &NumbersAnswer) -> u64 {
        answer.id
    }

    fn outcome(answer: &NumbersAnswer) -> Result<Result<u64, String>, ProtocolError> {
        answer.outcome()
    }
}

/// A draw waiting for its answer.
pub type Draw = Pending<NumbersAsk>;

/// Draws numbers from the conversations' sequences. Cloning shares the one
/// source.
#[derive(Clone)]
pub struct Numbers {
    asker: Asker<NumbersAsk>,
}

impl Numbers {
    /// A source whose draws arrive at the returned receiver, where the
    /// service's numbers stream, or a test, answers them.
    pub fn channel() -> (Self, mpsc::Receiver<Draw>) {
        let (asker, draws) = Asker::channel();
        (Self { asker }, draws)
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
        self.asker
            .ask(request)
            .await
            .ok_or_else(|| ServiceError::Numbers("the numbers stream has ended".into()))?
            .map_err(ServiceError::Numbers)
    }
}
