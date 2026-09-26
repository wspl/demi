//! The contract between the backend and a runner: the wire's messages, the
//! command manifest, and a managed runner's boot file; and where a Cloud
//! image puts what the runner reads. Both ends link this crate, so each is
//! defined once.

pub mod boot;
pub mod image;
pub mod manifest;
pub mod release;
pub mod values;
pub mod wire;
