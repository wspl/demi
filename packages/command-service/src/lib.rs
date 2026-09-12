//! HTTP/2 command services over caller-owned duplex transports.

pub use demi_command_protocol as protocol;

mod server;
mod stream;

pub use server::{Handler, InvocationContext, Output, serve};
pub use stream::{Input, ServiceError};
