//! Rust bindings generated from the shared Zod runner wire contract.

// Generated checks use uniform borrowed expressions across owned fields and references.
#[allow(
    clippy::needless_borrow,
    clippy::deref_addrof,
    clippy::len_zero,
    clippy::nonminimal_bool,
    clippy::collapsible_if,
    clippy::redundant_closure_call
)]
mod generated {
    include!(concat!(env!("OUT_DIR"), "/wire.rs"));
}
#[cfg(test)]
mod tests;
mod timestamp;

pub use generated::*;
pub use timestamp::Timestamp;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Visitor};
use std::fmt;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq)]
pub struct WireBytes(pub Vec<u8>);

impl Serialize for WireBytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            self.0.serialize(serializer)
        } else {
            serializer.serialize_bytes(&self.0)
        }
    }
}

impl<'de> Deserialize<'de> for WireBytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct BytesVisitor;
        impl<'de> Visitor<'de> for BytesVisitor {
            type Value = WireBytes;
            fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
                formatter.write_str("MessagePack binary bytes")
            }
            fn visit_bytes<E: serde::de::Error>(self, value: &[u8]) -> Result<Self::Value, E> {
                Ok(WireBytes(value.to_vec()))
            }
            fn visit_byte_buf<E: serde::de::Error>(self, value: Vec<u8>) -> Result<Self::Value, E> {
                Ok(WireBytes(value))
            }
        }
        deserializer.deserialize_bytes(BytesVisitor)
    }
}

#[derive(Debug, Error)]
pub enum WireError {
    #[error(transparent)]
    Decode(#[from] rmp_serde::decode::Error),
    #[error(transparent)]
    Encode(#[from] rmp_serde::encode::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("invalid runner message: {0}")]
    Invalid(String),
}

pub struct Outbound(Vec<u8>);

impl Outbound {
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

fn encode<T: Serialize>(message: &T) -> Result<Outbound, WireError> {
    Ok(Outbound(rmp_serde::to_vec_named(message)?))
}

pub fn decode(bytes: &[u8]) -> Result<Inbound, WireError> {
    let mut decoder = rmp_serde::Deserializer::new(std::io::Cursor::new(bytes));
    let message = Inbound::deserialize(&mut decoder)?;
    if decoder.position() != bytes.len() as u64 {
        return Err(WireError::Invalid("trailing MessagePack data".into()));
    }
    Ok(message)
}
