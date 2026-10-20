//! The messages stream (`crates/command-protocol/src/messages.rs`): the
//! handler tells the plugin of its package what changed in the state it
//! keeps for a conversation through its [`Messages`], and the runner answers
//! each message once its connection to the backend took it.

use tokio::sync::mpsc;

use crate::ServiceError;
use crate::asking::{Answered, Asked, Asker, Pending};
use demi_command_protocol::{MessageAnswer, PluginMessage, ProtocolError};

/// The messages stream's kind of request.
pub struct MessagesAsk;

impl Asked for MessagesAsk {
    type Request = PluginMessage;
    type Answer = MessageAnswer;
    type Reply = ();
    /// A message is answered at once.
    type Progress = std::convert::Infallible;
    const NAME: &'static str = "messages";

    fn with_id(message: PluginMessage, id: u64) -> PluginMessage {
        PluginMessage { id, ..message }
    }

    fn request_id(message: &PluginMessage) -> u64 {
        message.id
    }

    fn check(message: &PluginMessage) -> Result<(), ProtocolError> {
        message.validate()
    }

    fn answer(id: u64, result: Result<(), String>) -> MessageAnswer {
        MessageAnswer::new(id, result)
    }

    fn progress(_id: u64, progress: std::convert::Infallible) -> MessageAnswer {
        match progress {}
    }

    fn answer_id(answer: &MessageAnswer) -> u64 {
        answer.id
    }

    fn answered(answer: &MessageAnswer) -> Result<Answered<Self>, ProtocolError> {
        answer.outcome().map(Answered::Outcome)
    }
}

/// A message waiting for the runner to take it.
pub type Told = Pending<MessagesAsk>;

/// Tells the plugin of the service's package what changed. Cloning shares
/// the one stream.
#[derive(Clone)]
pub struct Messages {
    asker: Asker<MessagesAsk>,
}

impl Messages {
    /// A sender whose messages arrive at the returned receiver, where the
    /// service's messages stream, or a test, takes them.
    pub fn channel() -> (Self, mpsc::Receiver<Told>) {
        let (asker, told) = Asker::channel();
        (Self { asker }, told)
    }

    /// Sends `message` about `conversation` to the plugin, and returns once
    /// the runner's connection took it. A message sent before the runner
    /// opened the stream waits for it, and one sent while the runner has no
    /// connection waits for the next.
    pub async fn send(
        &self,
        conversation: &str,
        message: serde_json::Value,
    ) -> Result<(), ServiceError> {
        // Checked here, where the caller learns why, rather than where the
        // runner reads it and breaks the stream.
        let message = PluginMessage {
            id: 0,
            conversation: conversation.to_owned(),
            message,
        };
        message.validate()?;
        self.asker
            .ask(message, None)
            .await
            .ok_or_else(|| ServiceError::Messages("the messages stream has ended".into()))?
            .map_err(ServiceError::Messages)
    }
}
