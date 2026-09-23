//! The two-step decode of vendor payloads tagged by `type`: the tag first,
//! then the payload the tag names. An unregistered tag decodes to nothing, so
//! a vendor that adds an event does not break a stream Demi otherwise reads; a
//! registered tag with a malformed payload is an error that names the field;
//! a payload without a tag is an error.

use std::borrow::Cow;

use serde::{Deserialize, Deserializer, de};

/// A payload a vendor tags by `type`, of which a set of tags is registered.
/// [`tagged_wire!`](crate::tagged_wire) declares one.
pub trait TaggedWire: Sized {
    /// Decodes the payload of `tag`, or `None` when the set does not register
    /// `tag`.
    fn decode_payload<'de, D: Deserializer<'de>>(
        tag: &str,
        payload: D,
    ) -> Option<Result<Self, D::Error>>;
}

/// Declares an enum of vendor payloads keyed by their `type` tag, with one
/// variant per registered tag, and its [`TaggedWire`] decode.
///
/// ```ignore
/// tagged_wire! {
///     enum Event {
///         "message_start" => MessageStart(MessageStart),
///         "message_stop" => MessageStop(MessageStop),
///     }
/// }
/// ```
#[macro_export]
macro_rules! tagged_wire {
    (
        $(#[$meta:meta])*
        $vis:vis enum $name:ident {
            $( $(#[$variant_meta:meta])* $tag:literal => $variant:ident($payload:ty) ),* $(,)?
        }
    ) => {
        $(#[$meta])*
        $vis enum $name {
            $( $(#[$variant_meta])* $variant($payload), )*
        }

        impl $crate::wire::TaggedWire for $name {
            fn decode_payload<'de, D>(
                tag: &str,
                payload: D,
            ) -> ::core::option::Option<::core::result::Result<Self, D::Error>>
            where
                D: $crate::wire::__private::serde::Deserializer<'de>,
            {
                match tag {
                    $(
                        $tag => ::core::option::Option::Some(
                            <$payload as $crate::wire::__private::serde::Deserialize>::deserialize(payload)
                                .map(Self::$variant),
                        ),
                    )*
                    _ => ::core::option::Option::None,
                }
            }
        }
    };
}

/// Why a vendor payload could not be decoded; it names the offending field's
/// path.
#[derive(Debug, thiserror::Error)]
#[error("{0}")]
pub struct WireError(serde_path_to_error::Error<serde_json::Error>);

impl WireError {
    /// The path of the field that failed, such as `content_block.text`; `.`
    /// for the payload itself.
    pub fn path(&self) -> String {
        self.0.path().to_string()
    }
}

/// The tag of a payload.
#[derive(Deserialize)]
struct Tag<'a> {
    #[serde(rename = "type", borrow)]
    tag: Cow<'a, str>,
}

/// Decodes one vendor payload: `Ok(None)` for an unregistered tag.
pub fn decode_tagged<T: TaggedWire>(text: &str) -> Result<Option<T>, WireError> {
    let Tag { tag } = serde_path_to_error::deserialize(&mut serde_json::Deserializer::from_str(text))
        .map_err(WireError)?;
    let mut json = serde_json::Deserializer::from_str(text);
    let mut track = serde_path_to_error::Track::new();
    let payload = serde_path_to_error::Deserializer::new(&mut json, &mut track);
    match T::decode_payload(&tag, payload) {
        None => Ok(None),
        Some(Ok(decoded)) => Ok(Some(decoded)),
        Some(Err(error)) => Err(WireError(serde_path_to_error::Error::new(track.path(), error))),
    }
}

/// A tagged payload nested inside another, such as a stream event's content
/// block: `None` when its tag is not registered.
#[derive(Debug, Clone, PartialEq)]
pub struct Tagged<T>(pub Option<T>);

impl<'de, T: TaggedWire> Deserialize<'de> for Tagged<T> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = serde_json::Value::deserialize(deserializer)?;
        let Some(tag) = value.get("type") else {
            return Err(de::Error::missing_field("type"));
        };
        let tag = String::deserialize(tag).map_err(de::Error::custom)?;
        let mut track = serde_path_to_error::Track::new();
        let payload = serde_path_to_error::Deserializer::new(value, &mut track);
        match T::decode_payload(&tag, payload) {
            None => Ok(Self(None)),
            Some(Ok(decoded)) => Ok(Self(Some(decoded))),
            // The nested path goes into the message; the outer decode adds
            // the path up to this value.
            Some(Err(error)) => Err(de::Error::custom(serde_path_to_error::Error::new(
                track.path(),
                error,
            ))),
        }
    }
}

/// A field Demi only reports back to the user, such as a vendor error's
/// message: anything but a string reads as absent, so a malformed error
/// payload still surfaces as that error instead of as a protocol failure.
/// Use it with `#[serde(default)]`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReportedString(pub Option<String>);

impl ReportedString {
    pub fn as_deref(&self) -> Option<&str> {
        self.0.as_deref()
    }

    pub fn into_inner(self) -> Option<String> {
        self.0
    }
}

impl<'de> Deserialize<'de> for ReportedString {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match serde_json::Value::deserialize(deserializer)? {
            serde_json::Value::String(value) => Ok(Self(Some(value))),
            _ => Ok(Self(None)),
        }
    }
}
