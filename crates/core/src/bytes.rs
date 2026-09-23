//! Bytes in JSON: a base64 string.

use std::{borrow::Cow, fmt, ops::Deref};

use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::Bytes;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// Bytes that travel in JSON as a base64 string (RFC 4648, the standard
/// alphabet with padding). Cloning shares the bytes.
#[derive(Clone, PartialEq, Eq, Hash, Default)]
pub struct B64Bytes(Bytes);

impl B64Bytes {
    pub fn new(bytes: impl Into<Bytes>) -> Self {
        Self(bytes.into())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    pub fn into_bytes(self) -> Bytes {
        self.0
    }
}

impl From<Bytes> for B64Bytes {
    fn from(bytes: Bytes) -> Self {
        Self(bytes)
    }
}

impl From<Vec<u8>> for B64Bytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self(bytes.into())
    }
}

impl From<&'static [u8]> for B64Bytes {
    fn from(bytes: &'static [u8]) -> Self {
        Self(Bytes::from_static(bytes))
    }
}

impl Deref for B64Bytes {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl AsRef<[u8]> for B64Bytes {
    fn as_ref(&self) -> &[u8] {
        &self.0
    }
}

/// Shows the length rather than the bytes, which can be megabytes of media.
impl fmt::Debug for B64Bytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "B64Bytes({} bytes)", self.0.len())
    }
}

impl Serialize for B64Bytes {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for B64Bytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<'de, str>::deserialize(deserializer)?;
        let bytes = STANDARD
            .decode(text.as_bytes())
            .map_err(|error| de::Error::custom(format_args!("invalid base64: {error}")))?;
        Ok(Self(bytes.into()))
    }
}

impl JsonSchema for B64Bytes {
    fn inline_schema() -> bool {
        true
    }

    fn schema_name() -> Cow<'static, str> {
        "B64Bytes".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({ "type": "string", "contentEncoding": "base64" })
    }
}
