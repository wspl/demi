//! The user's small model (`models.md` § Small model): the model a small
//! job such as a conversation's title runs on. A model the user chose in
//! Settings › Agent while it is still offered, else Automatic: the first
//! tier any offered model reaches, Haiku, then GPT Sol, then Flash, at the
//! highest version of the first entry with a match, the conversation's own
//! entry first; else the conversation's own model. Its thinking is off
//! where the model can turn it off, else its lowest effort.

use std::cmp::Reverse;

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

/// The lowercase words of `text`, split at `-`, `_`, `.`, `/` and spaces.
fn words(text: &str) -> Vec<String> {
    text.to_lowercase()
        .split(['-', '_', '.', '/', ' '])
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Whether the ID or the name of `model` holds every word of `tier`.
fn matches_tier(model: &ProviderModel, tier: &[&str]) -> bool {
    [&model.id, &model.display_name].into_iter().any(|text| {
        let words = words(text);
        tier.iter().all(|wanted| words.iter().any(|word| word == wanted))
    })
}

fn unreleased(model: &ProviderModel) -> bool {
    [&model.id, &model.display_name]
        .into_iter()
        .flat_map(|text| words(text))
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


#[cfg(test)]
mod tests {
    use demi_shared_types::{AuthState, RuntimeState, Timestamp};
    use demi_web_api_protocol::ids::ProviderId;
    use demi_web_api_protocol::providers::{CatalogModel, UnavailableReason};
    use demi_web_api_protocol::settings::{HiddenModel, SmallModelSettings};

    use super::*;

    /// A model `id` named `name` that levels its thinking at `efforts` and
    /// can turn it off unless `no_off`.
    fn model(id: &str, name: &str, efforts: &[&str], no_off: bool) -> ProviderModel {
        ProviderModel {
            id: id.into(),
            display_name: name.into(),
            description: None,
            context_window: Some(200_000),
            output_limit: None,
            supports_tools: Some(true),
            accepted_extensions: None,
            supports_reasoning: Some(!efforts.is_empty()),
            supported_thinking_efforts: Some(efforts.iter().map(|effort| (*effort).to_owned()).collect()),
            can_disable_thinking: no_off.then_some(false),
            service_tiers: Vec::new(),
            default_service_tier_id: None,
            cost: None,
        }
    }

    fn entry(id: &str, models: &[&str]) -> CatalogProvider {
        CatalogProvider {
            provider_id: ProviderId::try_from(id).unwrap(),
            display_name: id.into(),
            cli_package: None,
            models: models
                .iter()
                .map(|id| {
                    let model = model(id, id, &["low", "high"], false);
                    CatalogModel {
                        selection: model.selection("x", None, None),
                        model,
                    }
                })
                .collect(),
            source_fetched_at: Timestamp::UNIX_EPOCH,
            stale: false,
            warnings: Vec::new(),
            auth: AuthState::Unknown { message: None },
            runtime: RuntimeState::Unknown { message: None },
            availability: Availability::Available {},
        }
    }

    /// The conversation's model `id` of the entry `provider`.
    fn on(provider: &str, id: &str) -> ModelSelection {
        model(id, id, &["medium", "high"], false).selection(
            provider,
            Some(ThinkingConfig::Effort {
                effort: "high".into(),
                summary: None,
            }),
            None,
        )
    }

    fn resolved(catalog: &[CatalogProvider], preferences: &Preferences, conversation: &ModelSelection) -> (String, String) {
        let small = small_model(catalog, preferences, conversation);
        (small.provider_id, small.model.id)
    }

    fn pair(provider: &str, model: &str) -> (String, String) {
        (provider.into(), model.into())
    }

    #[test]
    fn automatic_takes_the_first_tier_any_entry_offers() {
        let none = Preferences::default();
        let conversation = on("a", "opus");
        let cases: [(&[CatalogProvider], (String, String)); 6] = [
            // Haiku outranks a GPT Sol and a Flash, wherever they are.
            (
                &[entry("a", &["gemini-3.6-flash", "gpt-6.1-sol"]), entry("b", &["claude-haiku-5-5"])],
                pair("b", "claude-haiku-5-5"),
            ),
            // GPT Sol needs both words; `solar` is not `sol`.
            (
                &[entry("a", &["gpt-6-solar", "glm-4.7-flash"]), entry("b", &["gpt-6.1-sol"])],
                pair("b", "gpt-6.1-sol"),
            ),
            (&[entry("a", &["opus"]), entry("b", &["deepseek-v4.1-flash"])], pair("b", "deepseek-v4.1-flash")),
            // No tier: the conversation's own model.
            (&[entry("a", &["opus"]), entry("b", &["deepseek-v4"])], pair("a", "opus")),
            // The conversation's entry is looked at first, then the others in order.
            (
                &[entry("b", &["glm-4.7-flash"]), entry("c", &["gemini-9-flash"]), entry("a", &["deepseek-v4.1-flash"])],
                pair("a", "deepseek-v4.1-flash"),
            ),
            (
                &[entry("a", &["opus"]), entry("b", &["glm-4.7-flash"]), entry("c", &["gemini-9-flash"])],
                pair("b", "glm-4.7-flash"),
            ),
        ];
        for (catalog, expected) in cases {
            assert_eq!(resolved(catalog, &none, &conversation), expected, "{catalog:?}");
        }
    }

    #[test]
    fn a_tier_matches_the_display_name_as_well() {
        let mut gateway = entry("a", &[]);
        let model = model("acme/model-7", "GPT-5.6-Sol", &[], false);
        gateway.models.push(CatalogModel {
            selection: model.selection("a", None, None),
            model,
        });
        let small = small_model(&[gateway], &Preferences::default(), &on("a", "opus"));
        assert_eq!(small.model.id, "acme/model-7");
    }

    #[test]
    fn the_highest_version_wins_dates_left_out_and_a_release_before_a_preview() {
        let cases: [(&[&str], &str); 5] = [
            (&["claude-3-5-haiku-20241022", "claude-haiku-5-5", "claude-haiku-4-5"], "claude-haiku-5-5"),
            // A date is no version: 3.5 dated later is still below 4.5.
            (&["claude-haiku-4-5", "claude-3-5-haiku-2026-01-01"], "claude-haiku-4-5"),
            (&["gemini-3.6-flash-preview", "gemini-3.6-flash"], "gemini-3.6-flash"),
            (&["gemini-3.7-flash-exp", "gemini-3.6-flash"], "gemini-3.7-flash-exp"),
            // Equal in every way: the one the entry lists first.
            (&["x-flash-1", "y-flash-1"], "x-flash-1"),
        ];
        for (models, expected) in cases {
            let small = small_model(&[entry("a", models)], &Preferences::default(), &on("a", "opus"));
            assert_eq!(small.model.id, expected, "{models:?}");
        }
        // Versions are compared within one entry only: the first entry with
        // a match is the one, however high another's version.
        let small = small_model(
            &[entry("b", &["glm-4.7-flash"]), entry("c", &["deepseek-v9-flash"])],
            &Preferences::default(),
            &on("a", "opus"),
        );
        assert_eq!(small.model.id, "glm-4.7-flash");
    }

    #[test]
    fn thinking_is_off_else_the_lowest_effort_on_the_default_tier() {
        let mut catalog = entry("a", &[]);
        for model in [
            model("claude-haiku-5-5", "Claude Haiku 5.5", &["high", "low", "medium"], false),
            model("glm-4.7-flash", "GLM", &["high", "minimal"], true),
        ] {
            catalog.models.push(CatalogModel {
                selection: model.selection("a", None, None),
                model,
            });
        }
        let small = small_model(std::slice::from_ref(&catalog), &Preferences::default(), &on("a", "opus"));
        assert_eq!(small.thinking, Some(ThinkingConfig::Disabled {}));
        assert_eq!(small.service_tier_id, None);
        catalog.models.remove(0);
        let small = small_model(&[catalog], &Preferences::default(), &on("a", "opus"));
        assert_eq!(
            small.thinking,
            Some(ThinkingConfig::Effort {
                effort: "minimal".into(),
                summary: None
            })
        );
        // The conversation's own model, gone from its catalog, at its lowest effort.
        let small = small_model(&[entry("a", &[])], &Preferences::default(), &on("a", "opus"));
        assert_eq!(small.model.id, "opus");
        assert!(
            matches!(&small.thinking, Some(ThinkingConfig::Effort { effort, .. }) if effort == "medium"),
            "{:?}",
            small.thinking
        );
    }

    #[test]
    fn hidden_unavailable_and_other_entries_take_no_part() {
        let catalog = [entry("a", &["opus", "glm-4.7-flash"]), entry("b", &["claude-haiku-5-5"]), entry("c", &["gpt-6.1-sol"])];
        let conversation = on("a", "opus");
        let mut preferences = Preferences {
            hidden_providers: vec![ProviderId::try_from("b").unwrap()],
            ..Preferences::default()
        };
        assert_eq!(resolved(&catalog, &preferences, &conversation), pair("c", "gpt-6.1-sol"));
        preferences.hidden_models = vec![HiddenModel {
            provider_id: ProviderId::try_from("c").unwrap(),
            model_id: "gpt-6.1-sol".into(),
        }];
        assert_eq!(resolved(&catalog, &preferences, &conversation), pair("a", "glm-4.7-flash"));
        let mut unavailable = catalog.clone();
        unavailable[0].availability = Availability::Unavailable {
            reason: UnavailableReason::Authentication,
            message: "signed out".into(),
            missing: None,
        };
        // Nothing offered: the conversation's own model.
        assert_eq!(resolved(&unavailable, &preferences, &conversation), pair("a", "opus"));
        // Only the conversation's provider: Haiku elsewhere is not looked at.
        let same = Preferences {
            small_model_same_provider: Some(true),
            ..Preferences::default()
        };
        assert_eq!(resolved(&catalog, &same, &conversation), pair("a", "glm-4.7-flash"));
        assert_eq!(resolved(&catalog, &same, &on("c", "gpt-6.1-sol")), pair("c", "gpt-6.1-sol"));
    }

    #[test]
    fn a_chosen_model_holds_while_it_is_offered() {
        let catalog = [entry("a", &["opus"]), entry("b", &["claude-haiku-5-5", "sonnet"])];
        let conversation = on("a", "opus");
        let chosen = |provider: &str, model: &str, effort: Option<&str>| Preferences {
            small_model: Some(SmallModelSettings {
                provider_id: ProviderId::try_from(provider).unwrap(),
                model_id: model.into(),
                thinking_effort: effort.map(str::to_owned),
            }),
            ..Preferences::default()
        };
        let preferences = chosen("b", "sonnet", Some("high"));
        let small = small_model(&catalog, &preferences, &conversation);
        assert_eq!((small.provider_id.as_str(), small.model.id.as_str()), ("b", "sonnet"));
        assert!(matches!(&small.thinking, Some(ThinkingConfig::Effort { effort, .. }) if effort == "high"));
        // Gone, hidden, on another provider than the conversation's while
        // small requests stay on it, or without its effort: Automatic.
        let gone = [entry("a", &["opus"]), entry("b", &["claude-haiku-5-5"])];
        assert_eq!(resolved(&gone, &preferences, &conversation), pair("b", "claude-haiku-5-5"));
        let hidden = Preferences {
            hidden_models: vec![HiddenModel {
                provider_id: ProviderId::try_from("b").unwrap(),
                model_id: "sonnet".into(),
            }],
            ..preferences.clone()
        };
        assert_eq!(resolved(&catalog, &hidden, &conversation), pair("b", "claude-haiku-5-5"));
        let same = Preferences {
            small_model_same_provider: Some(true),
            ..preferences.clone()
        };
        assert_eq!(resolved(&catalog, &same, &conversation), pair("a", "opus"));
        assert_eq!(
            resolved(&catalog, &chosen("b", "sonnet", Some("max")), &conversation),
            pair("b", "claude-haiku-5-5")
        );
    }

    #[test]
    fn settings_show_automatic_as_for_the_first_offered_entry() {
        let catalog = [entry("a", &["glm-4.7-flash"]), entry("b", &["gemini-9-flash"])];
        let shown = automatic_small_model(&catalog, &Preferences::default()).unwrap();
        assert_eq!(shown.model.id, "glm-4.7-flash");
        let hidden = Preferences {
            hidden_providers: vec![ProviderId::try_from("a").unwrap()],
            ..Preferences::default()
        };
        assert_eq!(automatic_small_model(&catalog, &hidden).unwrap().model.id, "gemini-9-flash");
        // A chosen model does not change what Automatic shows.
        let chosen = Preferences {
            small_model: Some(SmallModelSettings {
                provider_id: ProviderId::try_from("b").unwrap(),
                model_id: "gemini-9-flash".into(),
                thinking_effort: None,
            }),
            ..Preferences::default()
        };
        assert_eq!(automatic_small_model(&catalog, &chosen).unwrap().model.id, "glm-4.7-flash");
        let same = Preferences {
            small_model_same_provider: Some(true),
            ..Preferences::default()
        };
        assert_eq!(automatic_small_model(&catalog, &same), None);
        assert_eq!(automatic_small_model(&[entry("a", &["opus"])], &Preferences::default()), None);
    }
}
