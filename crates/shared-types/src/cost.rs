//! What a vendor says a call cost (`usage-and-quota.md` § Usage ledger), in
//! the currency it names: a figure in another unit is converted to that
//! currency where the vendor documents the unit, and never between
//! currencies.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// An amount of a currency.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Cost {
    pub amount: f64,
    pub currency: Currency,
}

/// The currencies vendors bill in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Currency {
    Usd,
    Cny,
}

serde_plain::derive_display_from_serialize!(Currency);

impl fmt::Display for Cost {
    /// As a receipt writes it: `$0.25`, `¥1.80`; an amount under a cent
    /// keeps the digits that say it, `$0.0038`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let sign = match self.currency {
            Currency::Usd => "$",
            Currency::Cny => "¥",
        };
        if self.amount >= 0.01 || self.amount == 0.0 {
            write!(formatter, "{sign}{:.2}", self.amount)
        } else {
            let text = format!("{:.6}", self.amount);
            write!(formatter, "{sign}{}", text.trim_end_matches('0'))
        }
    }
}
