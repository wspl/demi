//! Bytes in JSON as a base64 string, and in a binary format as bytes; or,
//! in JSON whose bytes travel beside it ([`to_json_apart`]), as the index of
//! their buffer, as VS Code's extension host passes buffers:
//!
//! ```text
//! { "image": { "$$ref$$": 0 } }  +  buffers: [ <bytes of the image> ]
//! ```
//!
//! The JSON is serde_json's, which is human-readable, so a value inside an
//! internally tagged enum, which serde reads through a human-readable
//! buffer, takes the same path as any other.

use std::{borrow::Cow, cell::RefCell, fmt, ops::Deref};

use base64::{Engine, engine::general_purpose::STANDARD};
use bytes::Bytes;
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};

/// Bytes that travel in JSON as a base64 string (RFC 4648, the standard
/// alphabet with padding), and in a format that is not human-readable, such
/// as MessagePack, as its bytes. Cloning shares the bytes.
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

    /// The length of the bytes as the base64 text JSON carries them.
    pub fn base64_len(&self) -> u64 {
        let length = base64::encoded_len(self.0.len(), true)
            .expect("the base64 of bytes in memory has a length a usize holds");
        length as u64
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
        if !serializer.is_human_readable() {
            return self.0.serialize(serializer);
        }
        let taken = APART.with_borrow_mut(|apart| match apart {
            Some(Apart::Taking(buffers)) => {
                buffers.push(self.0.clone());
                Some(buffers.len() - 1)
            }
            _ => None,
        });
        match taken {
            Some(index) => {
                use serde::ser::SerializeMap as _;
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry(REF, &index)?;
                map.end()
            }
            None => serializer.serialize_str(&STANDARD.encode(&self.0)),
        }
    }
}

impl<'de> Deserialize<'de> for B64Bytes {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if !deserializer.is_human_readable() {
            return Bytes::deserialize(deserializer).map(Self);
        }
        deserializer.deserialize_any(Readable)
    }
}

/// The key of a buffer's index in JSON whose bytes travel beside it.
const REF: &str = "$$ref$$";

thread_local! {
    /// The buffers of the JSON this thread encodes or decodes now, while
    /// it does.
    static APART: RefCell<Option<Apart>> = const { RefCell::new(None) };
}

enum Apart {
    /// The bytes taken out so far, each at its index.
    Taking(Vec<Bytes>),
    /// The buffers the JSON's indexes name.
    Giving(Vec<Bytes>),
}

/// Reads bytes as base64 text, or, while JSON whose bytes travel beside it
/// is decoded, as the index of their buffer.
struct Readable;

impl<'de> de::Visitor<'de> for Readable {
    type Value = B64Bytes;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("base64 text, or the index of a buffer")
    }

    fn visit_str<E: de::Error>(self, text: &str) -> Result<B64Bytes, E> {
        let bytes = STANDARD
            .decode(text.as_bytes())
            .map_err(|error| E::custom(format_args!("invalid base64: {error}")))?;
        Ok(B64Bytes(bytes.into()))
    }

    fn visit_map<M: de::MapAccess<'de>>(self, mut map: M) -> Result<B64Bytes, M::Error> {
        let Some((key, index)) = map.next_entry::<Cow<'de, str>, usize>()? else {
            return Err(de::Error::custom("an empty object is no bytes"));
        };
        if key != REF || map.next_key::<de::IgnoredAny>()?.is_some() {
            return Err(de::Error::custom(format_args!("bytes are base64 text or {{\"{REF}\": index}}")));
        }
        let given = APART.with_borrow(|apart| match apart {
            Some(Apart::Giving(buffers)) => buffers.get(index).cloned(),
            _ => None,
        });
        given
            .map(B64Bytes)
            .ok_or_else(|| de::Error::custom(format_args!("no buffer {index} came with the JSON")))
    }
}

/// `value` as JSON whose bytes travel beside it: each [`B64Bytes`] in it as
/// the index of its buffer, and the buffers in that order.
pub fn to_json_apart<T: Serialize + ?Sized>(value: &T) -> serde_json::Result<(String, Vec<Bytes>)> {
    let _scope = Scope::enter(Apart::Taking(Vec::new()));
    let json = serde_json::to_string(value)?;
    let buffers = match APART.take() {
        Some(Apart::Taking(buffers)) => buffers,
        _ => Vec::new(),
    };
    Ok((json, buffers))
}

/// The value of `json`, whose bytes are `buffers` at the indexes it names
/// ([`to_json_apart`]); base64 text reads as ever.
pub fn from_json_apart<T: de::DeserializeOwned>(json: &[u8], buffers: Vec<Bytes>) -> serde_json::Result<T> {
    let _scope = Scope::enter(Apart::Giving(buffers));
    serde_json::from_slice(json)
}

/// The thread's buffers while one encoding or decoding runs; dropping it
/// ends them, a panic included.
struct Scope(Option<Apart>);

impl Scope {
    fn enter(apart: Apart) -> Self {
        Self(APART.replace(Some(apart)))
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        // An encoding inside another's, such as a value serialized to text
        // within it, gives the outer one its buffers back.
        APART.set(self.0.take());
    }
}

impl schemars::JsonSchema for B64Bytes {
    fn schema_name() -> Cow<'static, str> {
        "B64Bytes".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({ "type": "string", "contentEncoding": "base64" })
    }
}
