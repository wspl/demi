//! The Claude Code catalog (`models.md` § Directories): the `anthropic`
//! vendor of the models.dev document, its Claude models of version 4.6 or
//! later, flagship family first and newest first within a family.

use std::cmp::Ordering;

use demi_core::{ProviderModel, ProviderModelList};
use demi_provider::CatalogError;
use demi_provider::models_dev::{ModelsDevClient, ModelsDevSnapshot};

/// The models.dev vendor whose models the CLI runs.
const VENDOR: &str = "anthropic";

/// The oldest version the CLI runs.
const MINIMUM: ClaudeVersion = ClaudeVersion { major: 4, minor: 6 };

/// A Claude model's version, as its id names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ClaudeVersion {
    pub(crate) major: u32,
    pub(crate) minor: u32,
}

/// A fresh read of the catalog: the document asked for again.
pub(crate) async fn list(models_dev: &ModelsDevClient) -> Result<ProviderModelList, CatalogError> {
    let snapshot = models_dev
        .refreshed()
        .await
        .map_err(|error| CatalogError::Unavailable(error.to_string()))?;
    catalog(&snapshot)
}

/// The Claude models of the document's `anthropic` vendor.
pub(crate) fn catalog(snapshot: &ModelsDevSnapshot) -> Result<ProviderModelList, CatalogError> {
    let Some(mut list) = snapshot.vendor_models(VENDOR) else {
        return Err(CatalogError::Invalid(
            "models.dev does not list the anthropic vendor".into(),
        ));
    };
    let mut models = Vec::new();
    for mut model in std::mem::take(&mut list.models) {
        if !model.id.starts_with("claude-") {
            continue;
        }
        let Some(version) = claude_version(&model.id) else {
            list.warnings.push(format!(
                "Skipped Claude model with unparseable version: {}",
                model.id
            ));
            continue;
        };
        if version < MINIMUM {
            continue;
        }
        // The CLI's --effort option only levels thinking; it cannot turn it
        // off.
        model.can_disable_thinking = Some(false);
        models.push(model);
    }
    models.sort_by(order);
    list.models = models;
    Ok(list)
}

/// The version a Claude model id names: `claude-opus-4-8` is 4.8,
/// `claude-3-5-sonnet-20241022` is 3.5, and a date snapshot's date is not a
/// minor version, so `claude-sonnet-4-20250514` is 4.0.
pub(crate) fn claude_version(id: &str) -> Option<ClaudeVersion> {
    let parts: Vec<&str> = id.strip_prefix("claude-")?.split('-').collect();
    let number = |index: usize| -> Option<u32> {
        let part = parts.get(index)?;
        let digits = !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit());
        digits.then(|| part.parse().ok()).flatten()
    };
    let minor = |index: usize| {
        let is_date = parts.get(index).is_some_and(|part| part.len() == 8);
        if is_date {
            0
        } else {
            number(index).unwrap_or(0)
        }
    };
    if let Some(major) = number(0) {
        // A version before the family, as in claude-3-5-sonnet; a date there
        // is no minor version either.
        return Some(ClaudeVersion {
            major,
            minor: minor(1),
        });
    }
    let major = number(1)?;
    Some(ClaudeVersion {
        major,
        minor: minor(2),
    })
}

/// Opus first, then Sonnet, then Haiku, then others; newest version first
/// within a family; then by id.
fn order(left: &ProviderModel, right: &ProviderModel) -> Ordering {
    let family = |model: &ProviderModel| {
        let family = model
            .id
            .strip_prefix("claude-")
            .and_then(|rest| rest.split('-').next())
            .unwrap_or_default();
        match family {
            "opus" => 0,
            "sonnet" => 1,
            "haiku" => 2,
            _ => 3,
        }
    };
    family(left)
        .cmp(&family(right))
        .then_with(|| claude_version(&right.id).cmp(&claude_version(&left.id)))
        .then_with(|| left.id.cmp(&right.id))
}
