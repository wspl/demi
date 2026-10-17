//! What a provider failure keeps and what the product reads out of it
//! (`failures-and-recovery.md` § The failure record, § Reading a failure).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_with::rust::unwrap_or_skip;

use crate::{Nullable, Timestamp};

/// Where a provider failure came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FailureSource {
    /// A request the vendor answered with a failure status.
    Http,
    /// An error event inside a streamed response, or a frame Demi could not
    /// decode.
    Stream,
    /// No answer from the vendor.
    Transport,
    Unknown,
}

serde_plain::derive_display_from_serialize!(FailureSource);
serde_plain::derive_fromstr_from_deserialize!(FailureSource);

/// The diagnostics of a provider failure, saved with its `error` block. The
/// vendor's answer is in `upstream` exactly as it arrived: a stream's frame
/// text, or the JSON `{ status, headers, body }` of an HTTP failure; only the
/// provider that produced it reads it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProviderErrorDiagnostics {
    #[garde(skip)]
    pub source: FailureSource,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub client_request_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub provider_request_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub provider_response_id: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub provider_code: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "u16")]
    #[garde(skip)]
    pub http_status: Option<u16>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "unwrap_or_skip"
    )]
    #[schemars(with = "String")]
    #[garde(skip)]
    pub upstream: Option<String>,
}

/// What the provider that produced a failure record read out of it when the
/// record was shown. Sent beside the transcript, never stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct ProviderFailureFacts {
    /// The moment the vendor says the request can succeed again; null when
    /// it names none.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Timestamp>")]
    #[garde(skip)]
    pub retry_at: Option<Timestamp>,
    /// For a usage limit of an entry without rotation that has other
    /// accounts, the account with the most room now, which the dock offers
    /// to continue with (`providers.md` § Account rotation); omitted
    /// otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(dive)]
    pub continue_with: Option<ContinueWith>,
    /// For a request whose sign-in the vendor refused, the account it
    /// signed out while it still is, which the conversation names with
    /// Open Providers (`claude-code.md` § Accounts and sign-in); omitted
    /// otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(dive)]
    pub signed_out: Option<SignedOutAccount>,
    /// For a request to a port of the entry owner's device that the device
    /// had not exposed, that port, which the conversation offers to expose
    /// with Expose Port while the page holds it not exposed, and to try
    /// again once it is (`port-exposure.md` § Exposed ports); omitted
    /// otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(dive)]
    pub not_exposed: Option<NotExposedPort>,
    /// For a request to a port of a device that was offline, the device,
    /// which the failure's headline names (`port-exposure.md` § Exposed
    /// ports); omitted otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(dive)]
    pub device_offline: Option<OfflineDevice>,
}

/// A device a request named while it was offline: its id and its name, as
/// Settings shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct OfflineDevice {
    #[garde(length(min = 1))]
    pub device_id: String,
    #[garde(skip)]
    pub device: String,
}

/// A port of a device that a request named and the device has not exposed:
/// the device's id and name, as Settings shows it, and the port.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct NotExposedPort {
    #[garde(length(min = 1))]
    pub device_id: String,
    #[garde(skip)]
    pub device: String,
    #[garde(range(min = 1))]
    pub port: u16,
}

/// An account its vendor signed out: the provider entry's name and the
/// account's label, as Settings show them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct SignedOutAccount {
    #[garde(skip)]
    pub provider: String,
    #[garde(skip)]
    pub account: String,
}

/// An account a failed turn can continue with: its id and the label
/// Settings shows for it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase")]
pub struct ContinueWith {
    #[garde(length(min = 1))]
    pub credential_id: String,
    #[garde(skip)]
    pub label: String,
}
