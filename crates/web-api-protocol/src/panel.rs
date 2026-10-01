//! A conversation's work panel (`web-api.md` § Work panel state): the one
//! document the page saves whole and reads back as saved. The backend checks
//! its shape and bounds and never interprets a tab's `kind` or `data`.

use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The most tabs one work panel holds.
pub const PANEL_TABS_MAX: usize = 64;

/// The most bytes of one work panel's document, as the backend stores it.
pub const PANEL_BYTES_MAX: usize = 64 * 1024;

/// `GET/PUT /conversations/:id/panel`: what the panel selects, `"change"`,
/// `"file"` or a tab's id, and its tabs in the user's order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct WorkPanel {
    #[garde(length(min = 1))]
    pub selection: String,
    #[garde(length(max = PANEL_TABS_MAX), dive)]
    pub tabs: Vec<PanelTab>,
}

impl WorkPanel {
    /// The panel of a conversation that never saved one.
    pub fn empty() -> Self {
        Self {
            selection: "change".into(),
            tabs: Vec::new(),
        }
    }
}

/// One tab of the panel: what the page keeps for it, which only the page
/// reads.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct PanelTab {
    #[garde(length(min = 1))]
    pub id: String,
    #[garde(length(min = 1))]
    pub kind: String,
    #[garde(skip)]
    pub data: serde_json::Value,
}
