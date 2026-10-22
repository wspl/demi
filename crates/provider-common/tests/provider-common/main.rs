//! The provider contract's building blocks at their boundaries.
//!
//! The tests cover:
//!
//! - server-sent event framing over byte chunks;
//! - the two-step decode of tagged payloads;
//! - the Responses and Chat Completions streams over recorded frames;
//! - OAuth durations and the cache key of a long session id;
//! - the classification of a vendor's failure and the wait it names;
//! - the credential pool's refresh protocol;
//! - an account's quota snapshot;
//! - the models.dev document and the catalog of image and video models.
//!
//! The scripted runtime that the tests above providers run on has no tests of
//! its own: every test that scripts a model fails when it breaks.

mod chat_completions;
mod credentials;
mod failures;
mod media;
mod media_catalog;
mod models_dev;
mod oauth;
mod quota;
mod responses;
mod sse;
mod wire;
