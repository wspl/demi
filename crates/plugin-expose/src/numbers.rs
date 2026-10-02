//! The numbers the model knows exposes by (`expose.md` § The expose
//! record): `1`, `2`, … in order and never given twice, kept in the
//! plugin's value beside each expose's id, so the commands never print the
//! credential.

use std::collections::BTreeMap;

use demi_plugin_interface::{ExposeRecord, PluginError, PluginPort, PortFailure, PortRefusal};
use serde::{Deserialize, Serialize};

/// The value that holds the numbers.
const KEY: &str = "numbers";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Numbers {
    /// The number the next expose takes; it only grows.
    next: u64,
    /// Each live expose's number, by its id.
    exposes: BTreeMap<String, u64>,
}

impl Default for Numbers {
    fn default() -> Self {
        Self {
            next: 1,
            exposes: BTreeMap::new(),
        }
    }
}

/// A live expose and its number.
pub(crate) struct Numbered {
    pub number: u64,
    pub expose: ExposeRecord,
}

/// `exposes`, the user's live ones, each with its number. An expose without
/// one takes the next, the oldest first, and the numbers of exposes that
/// ended are let go; the next number never goes back, so none is given
/// twice. A write that another came before reads the numbers again.
pub(crate) async fn numbered(
    port: &PluginPort,
    exposes: Vec<ExposeRecord>,
) -> Result<Vec<Numbered>, PluginError> {
    loop {
        let stored = port.value(KEY).await?;
        let revision = stored.as_ref().map(|stored| stored.revision);
        let read = match stored {
            Some(stored) => serde_json::from_value::<Numbers>(stored.value).map_err(|error| {
                PluginError::failed(format!("the expose numbers do not read: {error}"))
            })?,
            None => Numbers::default(),
        };
        let mut numbers = read.clone();
        numbers
            .exposes
            .retain(|id, _| exposes.iter().any(|expose| expose.id.as_str() == id));
        let mut unnumbered: Vec<&ExposeRecord> = exposes
            .iter()
            .filter(|expose| !numbers.exposes.contains_key(expose.id.as_str()))
            .collect();
        unnumbered.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
        for expose in unnumbered {
            numbers.exposes.insert(expose.id.to_string(), numbers.next);
            numbers.next += 1;
        }
        if numbers != read {
            let value = serde_json::to_value(&numbers).map_err(PluginError::failed)?;
            match port.write_value(KEY, value, revision).await {
                Ok(_) => {}
                Err(PortFailure::Refused(PortRefusal::Conflict)) => continue,
                Err(failure) => return Err(failure.into()),
            }
        }
        return Ok(exposes
            .into_iter()
            .map(|expose| Numbered {
                number: numbers.exposes[expose.id.as_str()],
                expose,
            })
            .collect());
    }
}
