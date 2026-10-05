//! A conversation's work panel (`web-api.md` § Work panel state): its tabs,
//! which the backend keeps and changes a request of changes at a time, each
//! request counted by the panel's revision. The backend checks their shape and
//! bounds and never interprets a tab's `kind` or `data`.

use demi_shared_types::MAX_SAFE_INTEGER;
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// The most tabs one work panel holds.
pub const PANEL_TABS_MAX: usize = 64;

/// The most bytes of one work panel's tabs, as the backend stores them.
pub const PANEL_BYTES_MAX: usize = 64 * 1024;

/// The most characters of a tab's id.
pub const PANEL_TAB_ID_MAX: usize = 64;

/// `GET /conversations/:id/panel`: the tabs in order, and how many changes
/// made them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct WorkPanel {
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
    #[garde(dive)]
    pub tabs: Vec<PanelTab>,
}

impl WorkPanel {
    /// The panel of a conversation that never changed one.
    pub fn empty() -> Self {
        Self {
            revision: 0,
            tabs: Vec::new(),
        }
    }
}

/// One tab of the panel: its kind, and what the page and the kind's plugin
/// keep for it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct PanelTab {
    #[garde(length(min = 1, max = PANEL_TAB_ID_MAX))]
    pub id: String,
    #[garde(length(min = 1))]
    pub kind: String,
    #[garde(skip)]
    pub data: Map<String, Value>,
}

/// A new tab at `index`, after the others without one.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct CreatePanelTab {
    #[garde(length(min = 1, max = PANEL_TAB_ID_MAX))]
    pub id: String,
    #[garde(length(min = 1))]
    pub kind: String,
    #[garde(skip)]
    pub data: Map<String, Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[garde(range(max = PANEL_TABS_MAX))]
    pub index: Option<usize>,
}

/// `POST /conversations/:id/panel/changes`: changes applied in order, all
/// or none, as one change of the panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct PanelChanges {
    #[garde(length(min = 1), dive)]
    pub changes: Vec<PanelChange>,
}

/// What every change of the panel answers: its revision once the change is
/// in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct PanelRevision {
    #[garde(range(max = MAX_SAFE_INTEGER))]
    pub revision: u64,
}

/// One change of a panel, from the page or from the kind's plugin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema, Validate)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum PanelChange {
    /// Creates the tab.
    Create(#[garde(dive)] CreatePanelTab),
    /// Sets each field of `data` in the tab's `data`, removing those that
    /// are null, and leaves the other fields.
    Update {
        #[garde(length(min = 1, max = PANEL_TAB_ID_MAX))]
        id: String,
        #[garde(skip)]
        data: Map<String, Value>,
    },
    /// Removes the tab.
    #[serde(rename = "delete")]
    Remove {
        #[garde(length(min = 1, max = PANEL_TAB_ID_MAX))]
        id: String,
    },
    /// Moves the tab to `index` among the others.
    Move {
        #[garde(length(min = 1, max = PANEL_TAB_ID_MAX))]
        id: String,
        #[garde(range(max = PANEL_TABS_MAX))]
        index: usize,
    },
}

/// What the backend keeps of a panel: its tabs in order, and every id it
/// had, which is never used again.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, Validate)]
#[serde(deny_unknown_fields)]
pub struct PanelDocument {
    #[garde(dive)]
    pub tabs: Vec<PanelTab>,
    #[garde(skip)]
    pub retired: Vec<String>,
}

/// What a change did to which tab.
#[derive(Debug, Clone, PartialEq)]
pub enum PanelEffect {
    Created(PanelTab),
    Updated,
    /// The tab as it was when it was removed.
    Removed(PanelTab),
    Moved,
}

/// What applying a change came to.
#[derive(Debug, Clone, PartialEq)]
pub enum Applied {
    Effect(PanelEffect),
    /// Nothing to do: a tab the panel no longer has, an id it has or had,
    /// or a change that changes nothing.
    Nothing,
    /// A create past the most tabs or the most data.
    Full,
    /// An update past the most data.
    TooLarge,
}

/// Why a request of changes was refused, whole.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refused {
    /// A create past the most tabs or the most data.
    Full,
    /// An update past the most data.
    TooLarge,
}

/// Why a change was not applied, before its size is known.
enum Unapplied {
    Nothing,
    Full,
}

impl PanelDocument {
    /// Applies `changes` in order, all or none: each that has nothing to do
    /// is passed over, and one that does not fit leaves the document as it
    /// was and answers why.
    pub fn apply_all(&mut self, changes: Vec<PanelChange>) -> Result<Vec<PanelEffect>, Refused> {
        let mut next = self.clone();
        let mut effects = Vec::new();
        for change in changes {
            match next.apply(change) {
                Applied::Effect(effect) => effects.push(effect),
                Applied::Nothing => {}
                Applied::Full => return Err(Refused::Full),
                Applied::TooLarge => return Err(Refused::TooLarge),
            }
        }
        *self = next;
        Ok(effects)
    }

    /// Applies `change`, or leaves the document as it was when the change
    /// has nothing to do or does not fit.
    pub fn apply(&mut self, change: PanelChange) -> Applied {
        let mut next = self.clone();
        let effect = match next.change(change) {
            Ok(effect) => effect,
            Err(Unapplied::Nothing) => return Applied::Nothing,
            Err(Unapplied::Full) => return Applied::Full,
        };
        // The tabs serialize as JSON strings, arrays and objects with string
        // keys, which serde_json never refuses.
        let bytes = serde_json::to_string(&next.tabs)
            .expect("panel tabs serialize to JSON")
            .len();
        if bytes > PANEL_BYTES_MAX {
            return match effect {
                PanelEffect::Created(_) => Applied::Full,
                _ => Applied::TooLarge,
            };
        }
        *self = next;
        Applied::Effect(effect)
    }

    fn position(&self, id: &str) -> Option<usize> {
        self.tabs.iter().position(|tab| tab.id == id)
    }

    fn change(&mut self, change: PanelChange) -> Result<PanelEffect, Unapplied> {
        match change {
            PanelChange::Create(create) => {
                if self.position(&create.id).is_some() || self.retired.contains(&create.id) {
                    return Err(Unapplied::Nothing);
                }
                if self.tabs.len() >= PANEL_TABS_MAX {
                    return Err(Unapplied::Full);
                }
                let tab = PanelTab {
                    id: create.id,
                    kind: create.kind,
                    data: create.data,
                };
                let index = create.index.unwrap_or(self.tabs.len()).min(self.tabs.len());
                self.tabs.insert(index, tab.clone());
                Ok(PanelEffect::Created(tab))
            }
            PanelChange::Update { id, data } => {
                let index = self.position(&id).ok_or(Unapplied::Nothing)?;
                let tab = &mut self.tabs[index];
                let before = tab.data.clone();
                for (field, value) in data {
                    if value == Value::Null {
                        tab.data.remove(&field);
                    } else {
                        tab.data.insert(field, value);
                    }
                }
                if tab.data == before {
                    return Err(Unapplied::Nothing);
                }
                Ok(PanelEffect::Updated)
            }
            PanelChange::Remove { id } => {
                let index = self.position(&id).ok_or(Unapplied::Nothing)?;
                let tab = self.tabs.remove(index);
                self.retired.push(tab.id.clone());
                Ok(PanelEffect::Removed(tab))
            }
            PanelChange::Move { id, index } => {
                let from = self.position(&id).ok_or(Unapplied::Nothing)?;
                let to = index.min(self.tabs.len() - 1);
                if to == from {
                    return Err(Unapplied::Nothing);
                }
                let tab = self.tabs.remove(from);
                self.tabs.insert(to, tab);
                Ok(PanelEffect::Moved)
            }
        }
    }
}
