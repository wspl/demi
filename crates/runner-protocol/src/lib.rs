//! The contract between the backend and a runner: the wire's messages, the
//! command manifest, and a managed runner's boot file. Both ends link this
//! crate, so each is defined once.

pub mod boot;
pub mod manifest;
pub mod wire;
