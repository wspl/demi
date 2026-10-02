//! HTTP/2 command services over caller-owned duplex transports: the SDK of the
//! command wire that `demi_command_protocol` defines.

pub mod descriptors;
pub mod edits;
pub mod paths;

#[cfg(feature = "testing")]
pub mod testing;

mod client;
mod exchange;
mod launch;
mod numbers;
mod server;
mod stdio;
mod stream;
pub use stdio::serve_stdio;

pub use client::{Client, CommandInput, CommandOutput};
pub use exchange::{Exchange, ExchangeError, InputSource, OutputSink};
pub use launch::{Launch, launch_arguments};
pub use numbers::{Draw, Numbers, NumbersStream};
pub use server::{
    ConversationContext, Handler, InvocationContext, Output, serve, serve_cancellable,
};
pub use stream::{Input, ServiceError};
