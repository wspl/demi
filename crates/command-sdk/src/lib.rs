//! HTTP/2 command services over caller-owned duplex transports: the SDK of the
//! command wire that `demi_command_protocol` defines.

pub mod descriptors;
pub mod edits;
pub mod paths;

#[cfg(feature = "testing")]
pub mod testing;

mod artifacts;
mod asking;
mod client;
mod exchange;
mod launch;
mod numbers;
mod server;
mod stdio;
mod stream;
pub use stdio::serve_stdio;

pub use artifacts::{ArtifactPending, Artifacts, ArtifactsAsk};
pub use asking::{Answered, Asked, Pending, Reporter, RequestStream};
pub use client::{Client, CommandInput, CommandOutput};
pub use exchange::{Exchange, ExchangeError, InputSource, OutputSink};
pub use launch::{COMMAND_SERVICE, DATA, Launch};
pub use numbers::{Draw, Numbers, NumbersAsk};
pub use server::{
    ConversationContext, Handler, InvocationContext, MediumRefused, Output, serve,
    serve_cancellable,
};
pub use stream::{Input, ServiceError};
