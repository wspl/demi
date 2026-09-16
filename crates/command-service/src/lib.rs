//! HTTP/2 command services over caller-owned duplex transports.

pub mod edits;
pub mod integrity;
pub mod protocol;

mod client;
mod server;
mod stdio;
mod stream;
pub use stdio::serve_stdio;

pub use client::{Client, CommandInput, CommandOutput};
pub use server::{
    ConversationContext, Handler, InvocationContext, Output, serve, serve_cancellable,
};
pub use stream::{Input, ServiceError};
