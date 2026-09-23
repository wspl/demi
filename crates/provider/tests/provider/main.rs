//! The provider contract's building blocks at their boundaries: server-sent
//! event framing over byte chunks, the two-step decode of tagged payloads,
//! failure codes, records and their reading over a scripted vendor, the
//! scripted runtime that tests above providers run on, and credentials that
//! never print.

mod failures;
mod scripted;
mod secret;
mod sse;
mod wire;
