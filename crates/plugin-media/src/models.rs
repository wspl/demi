//! The `models` command of a media group (`crates/shared-types/src/output.rs`).
//!
//! - The models of the plugin's kind print as a table, newest first.
//! - One model prints as labelled lines, with its prices and its entries as
//!   tables under their keys.
//! - `--json` prints all of it as JSON.
//!
//! Notes, such as an entry whose models could not be listed, go to stderr.

use demi_host_interface::RpcInvocation;
use demi_plugin_interface::{EntryStatus, MediaModel, PluginError, PluginPort};
use demi_shared_types::MediaKind;
use demi_shared_types::thousands;
use demi_shared_types::output::{Cell, Fields, Table, instant, note, warning};
use schemars::JsonSchema;
use serde::Serialize;

use crate::words::{refusal_words, refused};

/// `models --json`: the list, or the one model shown.
#[derive(Serialize, JsonSchema)]
#[serde(untagged)]
pub enum ModelsOutput {
    List(Vec<ListedModel>),
    One(ListedModel),
}

/// A model as `models --json` prints it: every fact of it, with the entry
/// a call takes now.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ListedModel {
    #[serde(flatten)]
    pub model: MediaModel,
    /// The name of the entry a call takes now; null when none can.
    pub entry: Option<String>,
}

/// `demi <kind> models [<model>]`.
pub async fn models(invocation: &RpcInvocation, port: &PluginPort, kind: MediaKind, shown: Option<String>) -> Result<u8, PluginError> {
    let listing = port
        .media_models(kind)
        .await
        .map_err(|failure| refused(kind, failure, None, invocation))?;
    let command = format!("demi {}", invocation.path.join(" "));
    let mut notes = String::new();
    for unlisted in &listing.unlisted {
        notes.push_str(&warning(
            &command,
            &format!(
                "{}'s models could not be listed: {}",
                unlisted.provider,
                refusal_words(kind, &unlisted.refusal, None, invocation)
            ),
        ));
    }
    let listed: Vec<ListedModel> = listing
        .models
        .into_iter()
        .map(|model| ListedModel {
            entry: model
                .entries
                .iter()
                .find(|entry| entry.status == EntryStatus::Now)
                .map(|entry| entry.provider.clone()),
            model,
        })
        .collect();
    let rpc = port.rpc();
    if let Some(id) = shown {
        // An alias names the model it is an alias of, listed once.
        let Some(model) = listed
            .into_iter()
            .find(|model| model.model.id == id || model.model.aliases.iter().any(|alias| *alias == id))
        else {
            return Err(PluginError::failed(format!("no provider makes \"{id}\"; demi {kind} models lists them")));
        };
        unconfirmed(&mut notes, &command, std::slice::from_ref(&model));
        let text = if invocation.json {
            json_line(&ModelsOutput::One(model))
        } else {
            show(kind, &model)
        };
        rpc.stdout(text).await?;
        if !notes.is_empty() {
            rpc.stderr(notes).await?;
        }
        return Ok(0);
    }
    if let Some(none) = &listing.none {
        notes.push_str(&note(&command, &refusal_words(kind, none, None, invocation)));
    }
    unconfirmed(&mut notes, &command, &listed);
    let text = if invocation.json {
        json_line(&ModelsOutput::List(listed))
    } else {
        // An audio model's kinds are what a call chooses it by, so the
        // list says them.
        let audio = kind == MediaKind::Audio;
        let mut columns = vec!["id", "vendor", "released", "entry", "name"];
        if audio {
            columns.insert(1, "kinds");
        }
        let mut table = Table::new(columns);
        for listed in &listed {
            let model = &listed.model;
            let mut row: Vec<Cell> = vec![
                model.id.as_str().into(),
                model.vendor.as_deref().into(),
                model.released.as_deref().into(),
                listed.entry.as_deref().into(),
                model.name.as_deref().into(),
            ];
            if audio {
                let kinds: Vec<String> = model.audio.iter().flat_map(|audio| &audio.kinds).map(ToString::to_string).collect();
                row.insert(1, (!kinds.is_empty()).then(|| kinds.join(", ")).as_deref().into());
            }
            table.row(row);
        }
        table.render()
    };
    rpc.stdout(text).await?;
    if !notes.is_empty() {
        rpc.stderr(notes).await?;
    }
    Ok(0)
}

fn json_line(output: &ModelsOutput) -> String {
    let text = serde_json::to_string(output).expect("a listing encodes as JSON");
    format!("{text}\n")
}

/// A note, once for each entry, that its vendor does not confirm the model
/// that answers a call, which a call's line then says as *asked of*.
fn unconfirmed(notes: &mut String, command: &str, models: &[ListedModel]) {
    let mut named: Vec<&str> = Vec::new();
    for entry in models.iter().flat_map(|model| &model.model.entries) {
        if !entry.confirmed && !named.contains(&entry.provider.as_str()) {
            named.push(&entry.provider);
            notes.push_str(&note(command, &format!("{} does not confirm which model answers a call", entry.provider)));
        }
    }
}

fn joined(values: Option<&[String]>) -> String {
    match values {
        Some(values) if !values.is_empty() => values.join(", "),
        _ => "-".to_owned(),
    }
}

/// One model as labelled lines, the facts of its kind, its prices and its
/// entries as tables under their keys.
fn show(kind: MediaKind, listed: &ListedModel) -> String {
    let model = &listed.model;
    let mut fields = Fields::new();
    fields
        .field("ID", model.id.as_str())
        .field("Vendor", model.vendor.as_deref())
        .field("Released", model.released.as_deref())
        .field("Entry", listed.entry.as_deref())
        .field("Name", model.name.as_deref())
        .field("Alias of", model.alias_of.as_deref())
        .field("Aliases", joined(Some(&model.aliases)));
    match kind {
        MediaKind::Image => image_facts(&mut fields, model),
        MediaKind::Video => video_facts(&mut fields, model),
        MediaKind::Audio => audio_facts(&mut fields, model),
    }
    fields.field("Price page", model.price.as_ref().and_then(|price| price.page.as_deref()));
    let rates = model.price.as_ref().map(|price| &price.rates[..]).unwrap_or_default();
    if rates.is_empty() {
        fields.field("Prices", None::<&str>);
    } else {
        let mut prices = Table::new(["unit", "modality", "amount", "currency", "size", "quality", "resolution", "duration (s)"]);
        for rate in rates {
            prices.row::<Cell>([
                rate.unit.as_str().into(),
                rate.modality.as_deref().into(),
                rate.amount.to_string().into(),
                rate.currency.as_str().into(),
                rate.size.as_deref().into(),
                rate.quality.as_deref().into(),
                rate.resolution.as_deref().into(),
                rate.duration.into(),
            ]);
        }
        fields.table("Prices", &prices);
    }
    let mut entries = Table::new(["entry", "kind", "account", "price", "status"]);
    for entry in &model.entries {
        let price = match (entry.subscription, rates.is_empty()) {
            (true, _) => Some("included"),
            (false, false) => Some("listed"),
            (false, true) => None,
        };
        entries.row::<Cell>([
            entry.provider.as_str().into(),
            if entry.subscription { "subscription" } else { "api-key" }.into(),
            entry.account.as_deref().into(),
            price.into(),
            status_words(kind, &entry.status).into(),
        ]);
    }
    fields.table("Entries", &entries);
    if let Some(description) = &model.description {
        fields.text("Description", description);
    }
    fields.render()
}

/// What an image model takes: its sizes, shapes, edits and background.
fn image_facts(fields: &mut Fields, model: &MediaModel) {
    let image = model.image.clone().unwrap_or_default();
    let custom = image.custom_size.as_ref().map(|rule| {
        let mut said = vec!["any WxH".to_owned()];
        if let Some(multiple) = rule.multiple_of {
            said.push(format!("each edge a multiple of {multiple}"));
        }
        if let Some(edge) = rule.max_edge {
            said.push(format!("each edge at most {edge}"));
        }
        if let Some(ratio) = rule.max_ratio {
            said.push(format!("long:short at most {ratio}:1"));
        }
        match (rule.min_pixels, rule.max_pixels) {
            (Some(min), Some(max)) => said.push(format!("{min} to {max} pixels")),
            (None, Some(max)) => said.push(format!("at most {max} pixels")),
            (Some(min), None) => said.push(format!("at least {min} pixels")),
            (None, None) => {}
        }
        said.join("; ")
    });
    fields
        .field("Sizes", joined(image.sizes.as_deref()))
        .field("Custom size", custom)
        .field("Aspect ratios", joined(image.aspect_ratios.as_deref()))
        .field("Resolutions", joined(image.resolutions.as_deref()))
        .field("Edits", image.edits)
        .field("Max input images", image.max_input_images)
        .field("Transparent background", image.transparent);
}

/// What a video model takes: its durations, shapes, resolutions, frames,
/// references and sound.
fn video_facts(fields: &mut Fields, model: &MediaModel) {
    let video = model.video.clone().unwrap_or_default();
    let durations = durations(video.durations.as_deref(), video.min_duration, video.max_duration);
    fields
        .field("Durations", durations)
        .field("Aspect ratios", joined(video.aspect_ratios.as_deref()))
        .field("Resolutions", joined(video.resolutions.as_deref()))
        .field("First frame", video.first_frame)
        .field("Last frame", video.last_frame)
        .field("Max reference images", video.max_reference_images)
        .field("Sound", video.audio);
}

/// What an audio model makes and takes: its kinds, voices, languages,
/// instructions, lengths, lyrics, instrumentals, loops and formats.
fn audio_facts(fields: &mut Fields, model: &MediaModel) {
    let audio = model.audio.clone().unwrap_or_default();
    let kinds: Vec<String> = audio.kinds.iter().map(|kind| kind.to_string()).collect();
    let voices = match (&audio.voices, audio.voices_listed) {
        (Some(voices), _) if !voices.is_empty() => Some(voices.join(", ")),
        (_, Some(true)) => Some(format!("listed by demi audio voices --model {}", model.id)),
        _ => None,
    };
    let durations = durations(audio.durations.as_deref(), audio.min_duration, audio.max_duration);
    fields
        .field("Kinds", joined(Some(&kinds)))
        .field("Voices", voices)
        .field("Languages", joined(audio.languages.as_deref()))
        .field("Instructions", audio.instructions)
        .field("Longest text", audio.max_characters.map(|characters| format!("{} characters", thousands(u64::from(characters)))))
        .field("Durations", durations)
        .field("Lyrics", audio.lyrics)
        .field("Instrumental", audio.instrumental)
        .field("Loops", audio.loops)
        .field("Formats", joined(audio.formats.as_deref()));
}

/// Lengths as a model takes them: the only ones, or a range.
fn durations(only: Option<&[u32]>, min: Option<f64>, max: Option<f64>) -> Option<String> {
    match (only, min, max) {
        (Some(durations), _, _) if !durations.is_empty() => {
            Some(durations.iter().map(|seconds| format!("{seconds} s")).collect::<Vec<_>>().join(", "))
        }
        (_, Some(min), Some(max)) => Some(format!("{min} to {max} s")),
        (_, Some(min), None) => Some(format!("at least {min} s")),
        (_, None, Some(max)) => Some(format!("at most {max} s")),
        _ => None,
    }
}

/// Why an entry would or would not make a call now, in the list's words.
fn status_words(kind: MediaKind, status: &EntryStatus) -> String {
    match status {
        EntryStatus::Now => "now".to_owned(),
        EntryStatus::Next => "next".to_owned(),
        EntryStatus::UsedUp { until: Some(until) } => format!("used up until {}", instant(*until)),
        EntryStatus::UsedUp { until: None } => "used up".to_owned(),
        EntryStatus::Off => format!("{} off", kind.switch()),
        EntryStatus::NotSignedIn => "not signed in".to_owned(),
        EntryStatus::TurnedOff => "turned off in Settings › Providers".to_owned(),
    }
}
