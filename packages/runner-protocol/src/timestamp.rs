use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};

/// Millisecond precision matches the Host contract's JavaScript Date values.
#[derive(Debug, Clone, Copy)]
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
        let mut bytes = Vec::with_capacity(12);
        bytes.extend_from_slice(&nanos.to_be_bytes());
        bytes.extend_from_slice(&seconds.to_be_bytes());
        Extension((-1, bytes.into())).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let Extension((kind, bytes)) = Extension::deserialize(deserializer)?;
        if kind != -1 {
            return Err(D::Error::custom("expected MessagePack timestamp"));
        }
        let (seconds, nanos) = match bytes.len() {
            4 => (u32::from_be_bytes(bytes[..].try_into().unwrap()) as i64, 0),
            8 => {
                let value = u64::from_be_bytes(bytes[..].try_into().unwrap());
                ((value & 0x3_ffff_ffff) as i64, (value >> 34) as u32)
            }
            12 => (
                i64::from_be_bytes(bytes[4..].try_into().unwrap()),
                u32::from_be_bytes(bytes[..4].try_into().unwrap()),
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
            .and_then(|value| value.checked_add((nanos / 1_000_000) as i64))
            .ok_or_else(|| D::Error::custom("timestamp out of range"))?;
        Ok(Self(millis))
    }
}
