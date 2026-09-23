//! The provider contract's building blocks at their boundaries: server-sent
//! event framing over byte chunks, the two-step decode of tagged payloads,
//! failure codes, records and their reading over a scripted vendor, the
//! scripted runtime that tests above providers run on, credentials that
//! never print, the credential pool with its refresh protocol and account
//! operations, an account's quota snapshot, and the models.dev document.

mod credentials;
mod failures;
mod models_dev;
mod quota;
mod scripted;
mod secret;
mod sse;
mod wire;
