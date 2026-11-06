//! The starter lane: what a program's core asks of the program that opened
//! it, on the program's page connection to the core, and the program's
//! answers.
//!
//! ```text
//! core                                      program
//!   {"type":"notify","id":1,"message":{..}} ------->
//!                                   <------- {"type":"shown","id":1}
//! ```
//!
//! One starter lane is open for a core at a time. While none is, the core's
//! notifications reach no one.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::push::PushMessage;

/// What the core asks, each with an id its answer names.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum StarterRequest {
    /// Show `message` to the user, after the notifications before it.
    Notify { id: u32, message: PushMessage },
}

/// What the program answers: how the notification `id` went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum StarterAnswer {
    /// It goes to `to` of the places the program shows notifications in.
    Sending { id: u32, to: u32 },
    /// Every place that would show it shows its conversation already.
    Shown { id: u32 },
    /// Nowhere takes it.
    Nowhere { id: u32 },
}
