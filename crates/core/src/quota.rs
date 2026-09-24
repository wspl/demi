//! A subscription account's quota snapshot (`usage-and-quota.md` § Vendor
//! quota): the vendor's latest report of how much of the account's plan is
//! used. The backend stores it with the account, without the vendor's
//! payload, and reads it back, so it refuses unknown fields.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{Nullable, Timestamp};

/// The latest report of one account.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuotaSnapshot {
    #[garde(skip)]
    pub observed_at: Timestamp,
    /// Which path filled the snapshot last.
    #[garde(skip)]
    pub source: SnapshotSource,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<QuotaPlan>")]
    #[garde(dive)]
    pub plan: Option<QuotaPlan>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    #[garde(skip)]
    pub account_label: Option<String>,
    /// Each window as its own source said last; one the vendor does not
    /// report is absent.
    #[garde(dive)]
    pub windows: Vec<QuotaWindow>,
}

/// How a snapshot was filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotSource {
    /// A request to the vendor's usage endpoint.
    Probe,
    /// The quota a vendor reported beside a live inference response.
    Observation,
}

serde_plain::derive_display_from_serialize!(SnapshotSource);
serde_plain::derive_fromstr_from_deserialize!(SnapshotSource);

/// The account's plan: the vendor's id and the name the user reads.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuotaPlan {
    #[garde(length(chars, min = 1))]
    pub id: String,
    #[garde(skip)]
    pub label: String,
}

/// One limit the vendor meters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, garde::Validate)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuotaWindow {
    /// A stable id, such as `primary`, `five_hour` or `monthly`.
    #[garde(length(chars, min = 1))]
    pub id: String,
    /// The name the user knows the window by, such as `5-hour`.
    #[garde(skip)]
    pub label: String,
    /// The share used, from 0 to 100; null when unknown.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<f64>")]
    #[garde(range(min = 0.0, max = 100.0))]
    pub used_percent: Option<f64>,
    /// The amount used, in the window's unit, when the vendor reports it.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<f64>")]
    #[garde(skip)]
    pub used: Option<f64>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<f64>")]
    #[garde(skip)]
    pub limit: Option<f64>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<QuotaUnit>")]
    #[garde(skip)]
    pub unit: Option<QuotaUnit>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<Timestamp>")]
    #[garde(skip)]
    pub resets_at: Option<Timestamp>,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<QuotaSeverity>")]
    #[garde(skip)]
    pub severity: Option<QuotaSeverity>,
    /// What the window applies to when that is narrower than the account,
    /// such as one model.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<QuotaScope>")]
    #[garde(skip)]
    pub scope: Option<QuotaScope>,
}

/// The unit of a window's amounts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuotaUnit {
    Percent,
    Credits,
    /// Hundredths of a dollar.
    UsdMinor,
    Requests,
    Tokens,
}

serde_plain::derive_display_from_serialize!(QuotaUnit);
serde_plain::derive_fromstr_from_deserialize!(QuotaUnit);

/// How close a window is to its limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum QuotaSeverity {
    Normal,
    Warning,
    Critical,
}

serde_plain::derive_display_from_serialize!(QuotaSeverity);
serde_plain::derive_fromstr_from_deserialize!(QuotaSeverity);

/// The part of an account a window meters, such as one model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct QuotaScope {
    pub kind: String,
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub label: Option<String>,
}
