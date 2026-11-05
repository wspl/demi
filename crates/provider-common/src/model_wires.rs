//! How each model of an entry wants thinking sent and sent back
//! (`crates/provider-registry/src/registry.rs`).

use std::collections::HashMap;

use demi_shared_types::ModelWire;

/// The wire facts of an entry's models, by model id, which each protocol's
/// provider reads for the model a request names.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelWires(HashMap<String, ModelWire>);

impl ModelWires {
    pub fn new(models: HashMap<String, ModelWire>) -> Self {
        Self(models)
    }

    /// These facts, with `models`' in place of any of the same model.
    pub fn with(mut self, models: impl IntoIterator<Item = (String, ModelWire)>) -> Self {
        self.0.extend(models);
        self
    }

    /// The facts of model `model_id`; the defaults for a model the entry does
    /// not describe, such as one models.dev does not list.
    pub fn of(&self, model_id: &str) -> ModelWire {
        self.0.get(model_id).copied().unwrap_or_default()
    }
}
