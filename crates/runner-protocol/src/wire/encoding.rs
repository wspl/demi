//! How the wire carries bytes and times: MessagePack `bin` and the timestamp
//! extension, never arrays or strings.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error, de::Visitor};

/// Bytes that travel as MessagePack `bin`.
#[derive(Debug, Clone, PartialEq, Eq)]
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
            fn visit_bytes<E: Error>(self, value: &[u8]) -> Result<Self::Value, E> {
                Ok(WireBytes(value.to_vec()))
            }
            fn visit_byte_buf<E: Error>(self, value: Vec<u8>) -> Result<Self::Value, E> {
                Ok(WireBytes(value))
            }
        }
        deserializer.deserialize_bytes(BytesVisitor)
    }
}

/// A time in milliseconds since the Unix epoch, the precision of the Host
/// contract's JavaScript `Date` values. It travels as the MessagePack
/// timestamp extension (type -1) in its shortest form.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp(pub i64);

#[derive(Serialize, Deserialize)]
#[serde(rename = "_ExtStruct")]
struct Extension((i8, serde_bytes::ByteBuf));

impl Serialize for Timestamp {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
            return self.0.serialize(serializer);
        }
        let seconds = self.0.div_euclid(1000);
        let nanos = (self.0.rem_euclid(1000) * 1_000_000) as u32;
        // The MessagePack specification's forms: 32 bits of seconds, 30 bits
        // of nanoseconds with 34 of seconds, or 32 and 64 bits.
        let bytes = match u64::try_from(seconds) {
            Ok(seconds) if nanos == 0 && seconds <= u64::from(u32::MAX) => {
                (seconds as u32).to_be_bytes().to_vec()
            }
            Ok(seconds) if seconds < 1 << 34 => {
                ((u64::from(nanos) << 34) | seconds).to_be_bytes().to_vec()
            }
            _ => {
                let mut bytes = Vec::with_capacity(12);
                bytes.extend_from_slice(&nanos.to_be_bytes());
                bytes.extend_from_slice(&seconds.to_be_bytes());
                bytes
            }
        };
        Extension((-1, bytes.into())).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let Extension((kind, bytes)) = Extension::deserialize(deserializer)?;
        if kind != -1 {
            return Err(D::Error::custom("expected MessagePack timestamp"));
        }
        let (seconds, nanos) = match *bytes.as_slice() {
            [a, b, c, d] => (i64::from(u32::from_be_bytes([a, b, c, d])), 0),
            [a, b, c, d, e, f, g, h] => {
                let value = u64::from_be_bytes([a, b, c, d, e, f, g, h]);
                ((value & 0x3_ffff_ffff) as i64, (value >> 34) as u32)
            }
            [a, b, c, d, e, f, g, h, i, j, k, l] => (
                i64::from_be_bytes([e, f, g, h, i, j, k, l]),
                u32::from_be_bytes([a, b, c, d]),
            ),
            _ => return Err(D::Error::custom("invalid MessagePack timestamp length")),
        };
        if nanos >= 1_000_000_000 {
            return Err(D::Error::custom(
                "invalid MessagePack timestamp nanoseconds",
            ));
        }
        let millis = seconds
            .checked_mul(1000)
            .and_then(|value| value.checked_add(i64::from(nanos / 1_000_000)))
            .ok_or_else(|| D::Error::custom("timestamp out of range"))?;
        Ok(Self(millis))
    }
}
