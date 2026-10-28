//! The user's small model: the model a small job such as a conversation's title
//! runs on.
//!
//! The model is the first of these that applies:
//!
//! 1. A model the user chose in Settings › Agent, while it is still offered.
//! 2. Automatic: the first tier any offered model reaches, Haiku, then GPT Sol,
//!    then Flash, at the highest version of the first entry with a match, the
//!    conversation's own entry first.
//! 3. The conversation's own model.
//!
//! Its thinking is off where the model can turn it off, else its lowest effort.

use std::cmp::Reverse;

use demi_provider_common::model_words;

use demi_shared_types::{Model, ModelSelection, ProviderModel, THINKING_OFF, ThinkingCapability, ThinkingConfig};
use demi_web_api_protocol::providers::{Availability, CatalogProvider};
use demi_web_api_protocol::settings::Preferences;

/// The words a model's ID or name holds for each tier of Automatic, in
/// order: a tier matches a model one of whose two holds all its words.
const TIERS: [&[&str]; 3] = [&["haiku"], &["gpt", "sol"], &["flash"]];

/// The words that mark a model as not yet released, which loses to a
/// released model of the same version.
const UNRELEASED: [&str; 3] = ["preview", "beta", "exp"];

/// Thinking efforts from the least to the most; an effort a catalog names
/// outside this list sorts after them.
const EFFORT_ORDER: [&str; 7] = ["none", "minimal", "low", "medium", "high", "xhigh", "max"];

/// The small model of a request for the conversation that infers with
/// `conversation`, from `catalog`, every entry of the user in the order of
/// Settings › Providers, and the user's `preferences`.
pub fn small_model(
    catalog: &[CatalogProvider],
    preferences: &Preferences,
    conversation: &ModelSelection,
) -> ModelSelection {
    let offered = Offered {
        catalog,
        preferences,
        only: preferences
            .small_model_same_provider
            .unwrap_or(false)
            .then_some(conversation.provider_id.as_str()),
    };
    if let Some(chosen) = offered.chosen() {
        return chosen;
    }
    if let Some(automatic) = offered.automatic(&conversation.provider_id) {
        return automatic;
    }
    // The conversation's own model, which its catalog may no longer list.
    let listed = catalog
        .iter()
        .find(|entry| entry.provider_id.as_str() == conversation.provider_id)
        .and_then(|entry| {
            entry
                .models
                .iter()
                .find(|listed| listed.model.id == conversation.model.id)
        });
    match listed {
        Some(listed) => small_selection(&conversation.provider_id, &listed.model),
        None => ModelSelection {
            thinking: lowest_thinking(&conversation.model),
            service_tier_id: None,
            ..conversation.clone()
        },
    }
}

/// What Automatic resolves to where no conversation is known, as Settings ›
/// Agent shows it: as for a conversation on the first offered entry; none
/// when small requests stay on the conversation's provider, which the page
/// cannot know, or when no tier matches.
pub fn automatic_small_model(catalog: &[CatalogProvider], preferences: &Preferences) -> Option<ModelSelection> {
    if preferences.small_model_same_provider.unwrap_or(false) {
        return None;
    }
    let offered = Offered {
        catalog,
        preferences,
        only: None,
    };
    let first = offered.entries().next()?;
    offered.automatic(first.provider_id.as_str())
}

/// The entries and models a small request may use: the available entries
/// the user did not hide, and their models the user did not hide; `only`
/// the conversation's entry when small requests stay on its provider.
struct Offered<'a> {
    catalog: &'a [CatalogProvider],
    preferences: &'a Preferences,
    only: Option<&'a str>,
}

impl<'a> Offered<'a> {
    fn entries(&self) -> impl Iterator<Item = &'a CatalogProvider> + '_ {
        self.catalog.iter().filter(move |entry| {
            matches!(entry.availability, Availability::Available {})
                && !self.preferences.hides_provider(&entry.provider_id)
                && self.only.is_none_or(|only| entry.provider_id.as_str() == only)
        })
    }

    fn models(&self, entry: &'a CatalogProvider) -> impl Iterator<Item = &'a ProviderModel> + '_ {
        entry
            .models
            .iter()
            .map(|listed| &listed.model)
            .filter(move |model| !self.preferences.hides_model(&entry.provider_id, &model.id))
    }

    /// The model the user chose, while its entry and model are offered and
    /// it still offers the chosen effort.
    fn chosen(&self) -> Option<ModelSelection> {
        let chosen = self.preferences.small_model.as_ref()?;
        let entry = self
            .entries()
            .find(|entry| entry.provider_id == chosen.provider_id)?;
        let model = self.models(entry).find(|model| model.id == chosen.model_id)?;
        let thinking = model.thinking_for(chosen.thinking_effort.as_deref()).ok()?;
        Some(model.selection(entry.provider_id.as_str(), thinking, None))
    }

    /// The first tier's best model, looked for in the entry `own` first,
    /// then in the others in their order.
    fn automatic(&self, own: &str) -> Option<ModelSelection> {
        let mut entries: Vec<_> = self.entries().collect();
        // A stable sort keeps the others in their order.
        entries.sort_by_key(|entry| entry.provider_id.as_str() != own);
        TIERS.iter().find_map(|tier| {
            entries.iter().find_map(|entry| {
                let best = self
                    .models(entry)
                    .enumerate()
                    .filter(|(_, model)| matches_tier(model, tier))
                    .max_by_key(|(index, model)| {
                        (version(&model.id), !unreleased(model), Reverse(*index))
                    })?;
                Some(small_selection(entry.provider_id.as_str(), best.1))
            })
        })
    }
}

/// `model` of the entry `provider` with its thinking off where it can turn
/// it off, else at its lowest effort, and the vendor's default tier.
fn small_selection(provider: &str, model: &ProviderModel) -> ModelSelection {
    let thinking = if model.can_turn_thinking_off() {
        model.thinking_for(Some(THINKING_OFF)).ok().flatten()
    } else {
        model
            .efforts()
            .into_iter()
            .min_by_key(|effort| effort_rank(effort))
            .and_then(|effort| model.thinking_for(Some(&effort)).ok().flatten())
    };
    model.selection(provider, thinking, None)
}

/// The least thinking `model` offers when its catalog no longer lists it:
/// its lowest named effort. A budget model thinks only when asked, so no
/// configuration is its least.
fn lowest_thinking(model: &Model) -> Option<ThinkingConfig> {
    model.thinking.iter().find_map(|capability| {
        let (efforts, adaptive) = match capability {
            ThinkingCapability::Adaptive { efforts, .. } => (efforts, true),
            ThinkingCapability::Effort { efforts, .. } => (efforts, false),
            ThinkingCapability::Budget { .. } | ThinkingCapability::Disabled {} => return None,
        };
        let effort = efforts.iter().min_by_key(|effort| effort_rank(effort))?.clone();
        Some(if adaptive {
            ThinkingConfig::Adaptive { effort }
        } else {
            ThinkingConfig::Effort {
                effort,
                summary: None,
            }
        })
    })
}

fn effort_rank(effort: &str) -> usize {
    EFFORT_ORDER
        .iter()
        .position(|known| *known == effort)
        .unwrap_or(EFFORT_ORDER.len())
}

/// Whether the ID or the name of `model` holds every word of `tier`.
fn matches_tier(model: &ProviderModel, tier: &[&str]) -> bool {
    [&model.id, &model.display_name].into_iter().any(|text| {
        let words = model_words(text);
        tier.iter().all(|wanted| words.iter().any(|word| word == wanted))
    })
}

fn unreleased(model: &ProviderModel) -> bool {
    [&model.id, &model.display_name]
        .into_iter()
        .flat_map(|text| model_words(text))
        .any(|word| UNRELEASED.contains(&word.as_str()))
}

/// The version of a model ID: its numbers read left to right, a run of
/// eight digits or a `YYYY-MM-DD` date left out, so `claude-haiku-5-5` is
/// 5.5 and `claude-3-5-haiku-20241022` is 3.5.
fn version(id: &str) -> Vec<u64> {
    // Each run of digits with the byte offset just past it.
    let mut runs: Vec<(&str, usize, usize)> = Vec::new();
    let mut start = None;
    for (at, character) in id.char_indices().chain([(id.len(), ' ')]) {
        match (character.is_ascii_digit(), start) {
            (true, None) => start = Some(at),
            (false, Some(from)) => {
                runs.push((&id[from..at], from, at));
                start = None;
            }
            _ => {}
        }
    }
    let joined = |left: &(&str, usize, usize), right: &(&str, usize, usize)| {
        right.1 == left.2 + 1 && id.as_bytes()[left.2] == b'-'
    };
    let mut numbers = Vec::new();
    let mut index = 0;
    while index < runs.len() {
        let run = &runs[index];
        let date = runs.get(index..index + 3).is_some_and(|date| {
            date[0].0.len() == 4
                && date[1].0.len() == 2
                && date[2].0.len() == 2
                && joined(&date[0], &date[1])
                && joined(&date[1], &date[2])
        });
        if date {
            index += 3;
            continue;
        }
        if run.0.len() != 8 {
            // A run too long for a number is no version anyone compares.
            numbers.push(run.0.parse().unwrap_or(u64::MAX));
        }
        index += 1;
    }
    numbers
}
