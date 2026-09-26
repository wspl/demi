//! Exposes (`web-api.md` § Exposes, `expose.md` § The expose record): a
//! service on one of the user's devices under a public URL for an hour.

use demi_core::Timestamp;
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::{DeviceId, ExposeId};

/// Where an expose's traffic goes on its device: `host:port` exactly as
/// given, or a bare port, which means `127.0.0.1:<port>`. The host is any
/// name or address the device can resolve, an IPv6 address in brackets; the
/// port is 1 to 65535.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "String")]
#[schemars(with = "String")]
pub struct ExposeAddress(String);

/// Why a text is not an expose's address.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("must be host:port or a port, the port 1 to 65535")]
pub struct NotExposeAddress;

impl ExposeAddress {
    /// The address as the record keeps it, a bare port spelled out.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The host the device's runner connects to: an IPv6 address without
    /// its brackets.
    pub fn host(&self) -> &str {
        let (host, _) = split(&self.0).expect("an address holds its host and port");
        host.strip_prefix('[')
            .and_then(|inner| inner.strip_suffix(']'))
            .unwrap_or(host)
    }

    pub fn port(&self) -> u16 {
        let (_, port) = split(&self.0).expect("an address holds its host and port");
        port
    }
}

impl TryFrom<String> for ExposeAddress {
    type Error = NotExposeAddress;

    fn try_from(text: String) -> Result<Self, NotExposeAddress> {
        if !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit()) {
            let port = port(&text).ok_or(NotExposeAddress)?;
            return Ok(Self(format!("127.0.0.1:{port}")));
        }
        split(&text).ok_or(NotExposeAddress)?;
        Ok(Self(text))
    }
}

/// The host and port of `address`, split at its last colon, when the host
/// is one a header can carry and the port is one a socket connects to.
fn split(address: &str) -> Option<(&str, u16)> {
    let (host, port_text) = address.rsplit_once(':')?;
    let bracketed = host.starts_with('[') || host.ends_with(']');
    let host_ok = !host.is_empty()
        && host.bytes().all(|byte| byte.is_ascii_graphic() && byte != b'/')
        && (!bracketed || (host.len() > 2 && host.starts_with('[') && host.ends_with(']')))
        && (bracketed || !host.contains(':'));
    if !host_ok {
        return None;
    }
    Some((host, port(port_text)?))
}

fn port(text: &str) -> Option<u16> {
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<u16>().ok().filter(|port| *port > 0)
}

/// An expose as every surface shows it: its public `url` and when it ends.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExposeDto {
    pub id: ExposeId,
    pub device_id: DeviceId,
    pub address: ExposeAddress,
    pub url: String,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,
}

/// `GET /exposes`: the caller's live exposes, soonest expiry first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Exposes {
    pub exposes: Vec<ExposeDto>,
}

/// `{ expose }`: the answer of a creation and a renewal.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExposeAnswer {
    pub expose: ExposeDto,
}

/// `POST /exposes`: a service on one of the caller's connected devices.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct CreateExpose {
    #[garde(skip)]
    pub device_id: DeviceId,
    /// An `ExposeAddress` is valid once it is decoded.
    #[garde(skip)]
    pub address: ExposeAddress,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(text: &str) -> Result<ExposeAddress, NotExposeAddress> {
        ExposeAddress::try_from(text.to_owned())
    }

    #[test]
    fn an_address_is_a_bare_port_on_the_loopback_or_a_host_and_port_as_given() {
        let bare = address("5173").unwrap();
        assert_eq!((bare.as_str(), bare.host(), bare.port()), ("127.0.0.1:5173", "127.0.0.1", 5173));
        let named = address("dev.internal:8080").unwrap();
        assert_eq!((named.as_str(), named.host(), named.port()), ("dev.internal:8080", "dev.internal", 8080));
        let six = address("[::1]:3000").unwrap();
        assert_eq!((six.as_str(), six.host(), six.port()), ("[::1]:3000", "::1", 3000));
        for refused in [
            "", "0", "65536", "localhost", ":80", "host:", "host:0", "host:http", "::1:80", "[]:80", "a b:80",
            "a/b:80", "[::1:80",
        ] {
            assert_eq!(address(refused), Err(NotExposeAddress), "{refused}");
        }
    }
}
