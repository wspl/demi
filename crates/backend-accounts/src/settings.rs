//! Per-user preferences (`web-api.md` § User preferences): the check a
//! reported locale passes before it is saved, and how a patch merges into
//! what is saved.

use demi_command_protocol::CommandLocale;
use demi_web_api_protocol::settings::{
    ContextLimitChange, ContextLimits, Preferences, PreferencesPatch,
};
use icu_locale::{Locale, LocaleCanonicalizer};
use icu_time::zone::iana::IanaParserExtended;

/// A patch whose locale, when it has one, names a time zone the backend
/// knows, in its IANA spelling, and holds each language once as its
/// canonical BCP 47 tag, in the order the user's browser reported them.
#[derive(Debug)]
pub struct CheckedPatch(PreferencesPatch);

/// Why a reported locale is refused. Each names the field, as a refused body
/// does.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LocaleError {
    #[error("locale.timeZone: {0:?} is not a time zone the backend knows")]
    UnknownTimeZone(String),
    #[error("locale.languages[{index}]: {tag:?} is not a BCP 47 language tag")]
    MalformedTag { index: usize, tag: String },
}

pub fn check(mut patch: PreferencesPatch) -> Result<CheckedPatch, LocaleError> {
    if let Some(locale) = patch.locale.take() {
        patch.locale = Some(canonical(locale)?);
    }
    Ok(CheckedPatch(patch))
}

/// The locale with its time zone checked against ICU4X's IANA names, which
/// ignore case, and its tags canonicalized as a web browser's
/// `Intl.getCanonicalLocales` does: `zh-cn` becomes `zh-CN` and `iw` becomes
/// `he`, and a tag that repeats an earlier one goes.
fn canonical(locale: CommandLocale) -> Result<CommandLocale, LocaleError> {
    let zone = IanaParserExtended::new().parse(&locale.time_zone);
    if zone.time_zone.is_unknown() {
        return Err(LocaleError::UnknownTimeZone(locale.time_zone));
    }
    let canonicalizer = LocaleCanonicalizer::new_extended();
    let mut languages: Vec<String> = Vec::with_capacity(locale.languages.len());
    for (index, tag) in locale.languages.iter().enumerate() {
        let mut parsed = Locale::try_from_str(tag).map_err(|_| LocaleError::MalformedTag {
            index,
            tag: tag.clone(),
        })?;
        canonicalizer.canonicalize(&mut parsed);
        let canonical = parsed.to_string();
        if !languages.contains(&canonical) {
            languages.push(canonical);
        }
    }
    Ok(CommandLocale {
        time_zone: zone.normalized.to_owned(),
        languages,
    })
}

/// `preferences` with `patch` applied: a field the patch holds replaces the
/// saved one, a `null` shortcut removes that override, a context limit
/// changes that one model's, and everything the patch leaves out stays.
pub fn merge(mut preferences: Preferences, CheckedPatch(patch): CheckedPatch) -> Preferences {
    if let Some(appearance) = patch.appearance {
        let saved = &mut preferences.appearance;
        saved.theme = appearance.theme.or(saved.theme);
        saved.tone = appearance.tone.or(saved.tone);
        saved.accent = appearance.accent.or(saved.accent);
        saved.font_size = appearance.font_size.or(saved.font_size);
    }
    if let Some(shortcuts) = patch.shortcuts {
        let saved = &mut preferences.shortcuts;
        for (change, override_) in [
            (shortcuts.new, &mut saved.new),
            (shortcuts.sidebar, &mut saved.sidebar),
            (shortcuts.settings, &mut saved.settings),
            (shortcuts.search, &mut saved.search),
        ] {
            if let Some(keys) = change {
                *override_ = keys;
            }
        }
    }
    preferences.last_model = patch.last_model.or(preferences.last_model);
    preferences.last_project_host = patch.last_project_host.or(preferences.last_project_host);
    preferences.locale = patch.locale.or(preferences.locale);
    if let Some(change) = patch.context_limit {
        set_context_limit(&mut preferences.context_limits, change);
    }
    preferences
}

/// Stores the limit `change` names for its model, or removes the model's
/// limit, and with it its entry's map once that is empty, so a model the
/// user returned to its full window has no stored limit.
fn set_context_limit(limits: &mut ContextLimits, change: ContextLimitChange) {
    let ContextLimitChange {
        provider_id,
        model_id,
        tokens,
    } = change;
    match tokens {
        Some(tokens) => {
            limits
                .entry(provider_id)
                .or_default()
                .insert(model_id, tokens);
        }
        None => {
            let Some(models) = limits.get_mut(&provider_id) else {
                return;
            };
            models.remove(&model_id);
            if models.is_empty() {
                limits.remove(&provider_id);
            }
        }
    }
}
