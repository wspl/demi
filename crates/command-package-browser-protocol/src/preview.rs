//! The web preview's stream (`preview.md` § The stream): what the relay in
//! the user's page and the preview engine on the Host say to each other over
//! the `preview` user stream. It is framed as the live view's stream is: a
//! four-byte big-endian length of the rest, a one-byte kind, then the
//! payload. Control messages are JSON; bodies and socket messages are binary
//! frames behind a small header. The page and the engine ship in the same
//! release; there is no version.

use std::collections::BTreeMap;

use demi_shared_types::{DecodeError, Nullable};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The declared operation that serves the stream (`native-runtime.md`
/// § User streams).
pub const OPERATION: &str = "browser.preview";

/// The page method's operation that gives the top-level environment and
/// label of an address the user opens in a tab of their browser, waking a
/// stopped Cloud first as opening any tab does (`preview.md` § The stream).
pub const OPEN_OPERATION: &str = "browser.preview_open";

/// The stream's arguments: none; the relay and the engine speak over the
/// stream.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct PreviewInput {}

/// `browser.preview_open`: an address the user opens, and where previews
/// live, as the page's `hello` names it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct PreviewOpenInput {
    /// An `http:` or `https:` address.
    #[garde(length(chars, min = 1, max = MAX_URL_CHARS), custom(web_address))]
    pub url: String,
    #[garde(skip)]
    pub scheme: PreviewScheme,
    #[garde(length(chars, min = 1, max = 253))]
    pub domain: String,
    #[garde(length(chars, min = 1, max = 63))]
    pub namespace: String,
    #[garde(length(chars, min = 1, max = 256))]
    pub host: String,
}

fn web_address(value: &str, _: &()) -> garde::Result {
    if value.starts_with("http://") || value.starts_with("https://") {
        Ok(())
    } else {
        Err(garde::Error::new("is not an http or https address"))
    }
}

/// What `browser.preview_open` answers: the top-level environment of the
/// address and its label, which the page opens the boot page of.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PreviewOpened {
    pub label: String,
    pub environment: PreviewEnvironment,
    /// The preview origin of the label: `<scheme>://<namespace>--<label>.<domain>`.
    pub origin: String,
}

/// A frame's kind, the byte after its length: UTF-8 JSON of one message.
pub const CONTROL_FRAME: u8 = 1;
/// The relay's: [`BodyHeader`], then up to [`BODY_CHUNK_BYTES`] of a
/// request's body; an empty one ends it.
pub const REQUEST_BODY_FRAME: u8 = 2;
/// The engine's: [`BodyHeader`], then up to [`BODY_CHUNK_BYTES`] of a
/// response's body; an empty one ends it.
pub const CHUNK_FRAME: u8 = 3;
/// Either side's: [`SocketHeader`], then one WebSocket message.
pub const SOCKET_MESSAGE_FRAME: u8 = 4;
/// The largest frame after its length.
pub const MAX_FRAME_BYTES: usize = 16 * 1024 * 1024;
/// The most body bytes one frame carries.
pub const BODY_CHUNK_BYTES: usize = 256 * 1024;
/// The version of the preview domain's static files the page and the
/// engine name, `/__demi/v<N>/` (`preview.md` § The preview domain service).
pub const FILES_VERSION: u32 = 1;

/// A body frame's header: the request's id (u32, big-endian).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BodyHeader {
    pub id: u32,
}

impl BodyHeader {
    pub const BYTES: usize = 4;

    /// Appends the header to `bytes`.
    pub fn write(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.id.to_be_bytes());
    }

    /// Splits a body frame's payload into its header and data.
    pub fn split(payload: &[u8]) -> Result<(Self, &[u8]), DecodeError> {
        let Some((header, data)) = payload.split_first_chunk::<{ Self::BYTES }>() else {
            return Err(DecodeError::Invalid(
                "a body frame is shorter than its header".into(),
            ));
        };
        if data.len() > BODY_CHUNK_BYTES {
            return Err(DecodeError::Invalid(format!(
                "a body frame carries {} bytes, more than {BODY_CHUNK_BYTES}",
                data.len()
            )));
        }
        Ok((
            Self {
                id: u32::from_be_bytes(*header),
            },
            data,
        ))
    }
}

/// A socket message frame's header: the socket's id (u32, big-endian), then
/// 1 for a binary message or 0 for text, which is UTF-8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SocketHeader {
    pub id: u32,
    pub binary: bool,
}

impl SocketHeader {
    pub const BYTES: usize = 5;

    /// Appends the header to `bytes`.
    pub fn write(&self, bytes: &mut Vec<u8>) {
        bytes.extend_from_slice(&self.id.to_be_bytes());
        bytes.push(u8::from(self.binary));
    }

    /// Splits a socket message frame's payload into its header and message.
    pub fn split(payload: &[u8]) -> Result<(Self, &[u8]), DecodeError> {
        let Some((header, data)) = payload.split_first_chunk::<{ Self::BYTES }>() else {
            return Err(DecodeError::Invalid(
                "a socket message frame is shorter than its header".into(),
            ));
        };
        let (id, flag) = header.split_first_chunk::<4>().expect("five bytes");
        let binary = match flag {
            [0] => false,
            [1] => true,
            [other] => {
                return Err(DecodeError::Invalid(format!(
                    "a socket message's flag is {other}, not 0 or 1"
                )));
            }
            _ => unreachable!("one byte after the id"),
        };
        if !binary && std::str::from_utf8(data).is_err() {
            return Err(DecodeError::Invalid(
                "a text socket message is not UTF-8".into(),
            ));
        }
        Ok((
            Self {
                id: u32::from_be_bytes(*id),
                binary,
            },
            data,
        ))
    }
}

/// A document environment (`preview.md` § Addresses and labels): its logical
/// origin, the logical top-level site of its frame tree, and whether an
/// ancestor of its frame is cross-site to it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct PreviewEnvironment {
    /// `http://localhost:5173`, or `null` for an opaque origin.
    #[garde(length(chars, min = 1, max = 2048))]
    pub origin: String,
    /// `http://localhost`, a scheme and a registrable domain.
    #[garde(length(chars, min = 1, max = 2048))]
    pub top: String,
    #[garde(skip)]
    pub cross: bool,
}

/// What the user's browser says about itself, as its own requests would
/// (`preview.md` § Upstream requests, § Mobile).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewClient {
    #[garde(length(chars, min = 1, max = 1024))]
    pub user_agent: String,
    /// `sec-ch-ua`, as the browser builds it from its brands.
    #[garde(length(chars, max = 1024))]
    pub brands: String,
    #[garde(skip)]
    pub mobile: bool,
    /// The platform's name, without quotes: `macOS`, `Android`.
    #[garde(length(chars, max = 64))]
    pub platform: String,
    #[garde(length(chars, max = 1024))]
    pub accept_language: String,
}

/// One header, in the order the browser gave them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(deny_unknown_fields)]
pub struct PreviewHeader {
    #[garde(length(chars, min = 1, max = 256))]
    pub name: String,
    #[garde(length(chars, max = 65_536))]
    pub value: String,
}

closed_set! {
    /// The preview domain's scheme, as the backend decided it.
    pub enum PreviewScheme {
        Https = "https",
        Http = "http",
    }
}

closed_set! {
    /// A request's mode, as the forwarder's `Request.mode` gives it.
    pub enum PreviewMode {
        Navigate = "navigate",
        Cors = "cors",
        NoCors = "no-cors",
        SameOrigin = "same-origin",
    }
}

closed_set! {
    /// A request's credentials mode.
    pub enum PreviewCredentials {
        Omit = "omit",
        SameOrigin = "same-origin",
        Include = "include",
    }
}

/// The most headers a request or a response carries.
pub const MAX_HEADERS: usize = 512;
/// The longest address: Chrome's limit on a URL.
pub const MAX_URL_CHARS: usize = 2 * 1024 * 1024;

/// A request the forwarder handed the relay, with real addresses, and who
/// started it as the relay resolved it (`preview.md` § The preview engine).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreviewRequest {
    /// The real address, with the engine's `__demi_` parameters the
    /// rewriter added.
    #[garde(length(chars, min = 1, max = MAX_URL_CHARS))]
    pub url: String,
    #[garde(length(chars, min = 1, max = 64))]
    pub method: String,
    /// The headers the forwarder saw: the page's own, and `Accept`.
    #[garde(length(max = MAX_HEADERS), dive)]
    pub headers: Vec<PreviewHeader>,
    /// Whether `request_body` frames follow, on the engine's pulls.
    #[garde(skip)]
    pub body: bool,
    #[garde(skip)]
    pub mode: PreviewMode,
    /// The fetch destination; empty for `fetch` and XHR.
    #[garde(length(chars, max = 32))]
    pub destination: String,
    #[garde(skip)]
    pub credentials: PreviewCredentials,
    /// The real referrer, under the page's policy; empty for none.
    #[garde(length(chars, max = MAX_URL_CHARS))]
    pub referrer: String,
    /// The request's referrer policy; empty for the default.
    #[garde(length(chars, max = 64))]
    pub referrer_policy: String,
    #[garde(skip)]
    pub keepalive: bool,
    /// The environment that started it: the receiving one for a
    /// subresource, the kept request's or the referrer's for a navigation;
    /// null when unknown, which counts as cross-site.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<PreviewEnvironment>")]
    #[garde(dive)]
    pub initiator: Option<PreviewEnvironment>,
    /// The user's own navigation: a preview tab's first load, or an address
    /// typed into its address bar.
    #[garde(skip)]
    pub user: bool,
}

/// What the relay sends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PreviewRelayMessage {
    /// First: where previews live, which the engine's labels and rewriting
    /// name, as the backend's product state carries it.
    Hello {
        /// `https`, or `http` for a development domain.
        #[garde(skip)]
        scheme: PreviewScheme,
        /// The preview domain, with a port when it is not the scheme's
        /// default: `demi-preview.dev`.
        #[garde(length(chars, min = 1, max = 253))]
        domain: String,
        /// The deployment's namespace.
        #[garde(length(chars, min = 1, max = 63))]
        namespace: String,
        /// The Host the labels name.
        #[garde(length(chars, min = 1, max = 256))]
        host: String,
    },
    /// A request of a preview document, received by `environment`, the
    /// environment the relay bound the forwarder's channel to.
    Request {
        #[garde(skip)]
        id: u32,
        #[garde(dive)]
        environment: PreviewEnvironment,
        #[garde(dive)]
        request: PreviewRequest,
        #[garde(dive)]
        client: PreviewClient,
    },
    /// Send the response body's next chunk.
    Pull {
        #[garde(skip)]
        id: u32,
    },
    /// The browser gave up on the request.
    Cancel {
        #[garde(skip)]
        id: u32,
    },
    /// A preview document's WebSocket.
    SocketOpen {
        #[garde(skip)]
        id: u32,
        #[garde(dive)]
        environment: PreviewEnvironment,
        /// The real `ws:` or `wss:` address.
        #[garde(length(chars, min = 1, max = MAX_URL_CHARS))]
        url: String,
        #[garde(length(max = 64), inner(length(chars, min = 1, max = 256)))]
        protocols: Vec<String>,
        #[garde(dive)]
        client: PreviewClient,
    },
    /// The page closed its socket.
    SocketClose {
        #[garde(skip)]
        id: u32,
        #[garde(range(min = 1000, max = 4999))]
        code: u16,
        #[garde(length(max = 123))]
        reason: String,
    },
}

impl PreviewRelayMessage {
    /// Decodes a control frame's payload from the relay.
    pub fn decode(payload: &[u8]) -> Result<Self, DecodeError> {
        demi_shared_types::decode_slice(payload)
    }
}

/// What the engine sends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PreviewEngineMessage {
    /// The head of an answer; its body follows on the relay's pulls.
    Response {
        #[garde(skip)]
        id: u32,
        #[garde(range(min = 100, max = 599))]
        status: u16,
        #[garde(length(max = MAX_HEADERS), dive)]
        headers: Vec<PreviewHeader>,
        /// The labels the answer's rewriting computed, each with its
        /// environment, for the relay to register.
        #[garde(dive)]
        labels: BTreeMap<String, PreviewEnvironment>,
    },
    /// Send the request body's next chunk.
    Pull {
        #[garde(skip)]
        id: u32,
    },
    /// The request failed before or during its answer; the forwarder answers
    /// a network error.
    Failed {
        #[garde(skip)]
        id: u32,
        #[garde(skip)]
        reason: String,
    },
    /// The socket is open upstream.
    SocketOpened {
        #[garde(skip)]
        id: u32,
        #[garde(skip)]
        protocol: String,
        #[garde(skip)]
        extensions: String,
    },
    /// The socket closed: upstream closed it (its code and reason), or it
    /// failed (1006, which the page's socket reports as an error first).
    SocketClose {
        #[garde(skip)]
        id: u32,
        #[garde(skip)]
        code: u16,
        #[garde(skip)]
        reason: String,
    },
}

impl PreviewEngineMessage {
    /// Decodes a control frame's payload from the engine.
    pub fn decode(payload: &[u8]) -> Result<Self, DecodeError> {
        demi_shared_types::decode_slice(payload)
    }
}
