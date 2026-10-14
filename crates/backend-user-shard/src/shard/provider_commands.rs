//! The product's `demi providers` group (`models.md` § Managing providers
//! from a conversation): the agent lists and shows the user's provider
//! entries with their models, adds, edits and removes entries, and searches
//! models.dev to add, edit and remove the models of an entry's configured
//! list. A change goes through the settings page's path, with its checks
//! and its words (`provider_settings`), and needs the user's permission,
//! the category [`MANAGE`]. No command takes a key as an argument or
//! prints one: `add` and `edit` read a key from stdin with
//! `--api-key-stdin` alone.

use std::rc::Rc;

use demi_backend_host_access::host_commands::conversation_of;
use demi_backend_host_access::root_of;
use demi_backend_providers::vault::entries::{EntryCredential, ProviderEntry};
use demi_host_interface::{Call, GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_agent_tools::EFFORT_OFF;
use demi_shared_types::{FileExtension, ModelSelection, WireApi, model_reads_images};
use demi_web_api_protocol::auth::UserDto;
use demi_web_api_protocol::error::problems;
use demi_web_api_protocol::providers::{
    Availability, ConfiguredModel, ConfiguredModels, CreateProvider,
    CredentialKind, ModelMatch, ModelSearchResults, ProviderPatch, SetupPart,
};
use demi_web_api_protocol::text::{EndpointUrl, Trimmed};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Shard;
use super::agent_profiles::Scope;
use crate::conversation::organize::verb;
use crate::conversation::product::entries;
use crate::provider_settings::ProviderRefusal;

/// The permission category of the commands that change the user's entries.
pub(crate) const MANAGE: &str = "providers.manage";

const MANAGE_ACTION: &str = "manage providers";

const MANAGE_DESCRIPTION: &str = "Add, change and remove your provider entries and their models, which every conversation of yours can use.";

const SUMMARY: &str = "The user's provider entries and their models: list and show them, add, edit and remove entries, and search models.dev to add models to an entry's list. A change needs the user's permission and reaches every conversation.";

/// The group's entry in the model's capability index (`system-prompt.md`
/// § Capability index).
const ENTRY: &str = "Lists the user's provider entries and the models each offers, the ones the model menu and subagent profiles can use, with what an entry still needs; adds, renames and removes entries, and adds models to an entry's list, filled from a models.dev search. Use it when the user asks which models they have, or to add a provider or a model. It never takes a key as an argument: the user adds keys in Settings, or a key is piped in with --api-key-stdin.";

const LIST_SUMMARY: &str = "List each provider entry by its name in Settings, its kind and its state (Ready, or what it still needs), then its models: id, name, context window, efforts (* marks the default; off when thinking can be turned off), service tiers and whether it reads images. The model you run on is marked. --all adds the entries and models the user turned off. A read.";

const SHOW_SUMMARY: &str = "Show every setting of one entry: name, kind, vendor or family and wire, endpoint, whether a key is saved (never the key), accounts by label, state, and each model with every field. A read.";

const ADD_SUMMARY: &str = "Add a provider entry: --vendor, a models.dev vendor; --family alone, the family's own vendor; or --family with --wire and --base-url, a compatible API. Without --name it is named after its vendor or family, or Compatible API, with (2), (3) when that name is taken. A vendor's entry without a key joins the list incomplete until the user adds the key in Settings > Providers; a compatible API's key is optional. Needs the user's permission.";

const EDIT_SUMMARY: &str = "Change an entry's name, its endpoint, or its key with --api-key-stdin. Needs the user's permission.";

const REMOVE_SUMMARY: &str = "Remove an entry with its accounts, as Settings does. Needs the user's permission.";

const MODELS_SUMMARY: &str = "The models of an entry's own list: search models.dev, and add, edit and remove models. A subscription entry lists its own models.";

const SEARCH_SUMMARY: &str = "Search models.dev for models whose name or id holds each word, newest first, at most 20: each match's <vendor>/<model id>, name, context window, output limit, efforts and the files it reads. `demi providers models add` takes the <vendor>/<model id>. A read.";

const MODEL_ADD_SUMMARY: &str = "Add a model to an entry's list, filled from a search match; the options set or override fields, and without a match they are the whole model. An entry that follows its vendor's list starts a list of its own from the vendor's models and this one, and no longer follows models.dev. Needs the user's permission.";

const MODEL_EDIT_SUMMARY: &str = "Change the fields of a model of an entry's list that the options name. Needs the user's permission.";

const MODEL_REMOVE_SUMMARY: &str = "Remove a model from an entry's list. Needs the user's permission.";

const ALL: &str = "Also list the entries and models the user turned off in Settings > Providers.";
const VENDOR: &str = "A models.dev vendor's id, such as google, as Settings > Providers offers it.";
const FAMILY: &str = "The family whose protocol the entry speaks: openai, anthropic or google. With --base-url the entry is a compatible API; without, the family's own vendor's.";
const WIRE: &str = "The wire of an openai endpoint, responses or chat-completions; responses without it.";
const ENTRY_ARG: &str = "The entry's name or id, as `demi providers list` shows them.";
const NAME: &str = "The entry's name, 1 to 80 characters, as Settings shows it.";
const BASE_URL: &str = "The API's address, an http or https URL such as https://api.example.com/v1.";
const API_KEY_STDIN: &str = "Read the entry's key from stdin, one line; pipe it in, never type it into a command.";
const API_KEY: &str = "The key, read from stdin with --api-key-stdin.";
const MODEL_ID: &str = "The model's id as the entry's vendor names it.";
const MODEL_NAME: &str = "The model's name in the model menu; its id without one.";
const CONTEXT: &str = "The context window in tokens.";
const OUTPUT: &str = "The output limit in tokens, at most the context window, or none.";
const EFFORTS: &str = "The thinking efforts, comma-separated, the default first, such as low,medium,high; none for a model without thinking.";
const EXTENSIONS: &str = "The file types the model reads, comma-separated, such as png,jpg,pdf; none for no file, unknown when not known.";
const FAST_TIER: &str = "The service tier that is the model's Fast, such as priority, or none.";

/// The input of `list`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListArgs {
    all: Option<bool>,
}

/// The input of `show` and `remove`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EntryArgs {
    entry: String,
}

/// The input of `add`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct AddArgs {
    vendor: Option<String>,
    family: Option<String>,
    wire: Option<String>,
    base_url: Option<String>,
    name: Option<String>,
    api_key_stdin: Option<bool>,
    api_key: Option<String>,
}

/// The input of `edit`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct EditArgs {
    entry: String,
    name: Option<String>,
    base_url: Option<String>,
    api_key_stdin: Option<bool>,
    api_key: Option<String>,
}

/// The input of `models search`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    /// The words a model's name or id holds, such as gemini flash.
    #[schemars(length(min = 1))]
    query: Vec<String>,
}

/// The input of `models add`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct ModelAddArgs {
    entry: String,
    /// A search match's <vendor>/<model id>, such as google/gemini-3.8-flash.
    model: Option<String>,
    #[serde(flatten)]
    fields: ModelFields,
}

/// The input of `models edit`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct ModelEditArgs {
    entry: String,
    /// The id of the model in the entry's list.
    model: String,
    #[serde(flatten)]
    fields: ModelFields,
}

/// The fields of a configured model the options set.
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
struct ModelFields {
    id: Option<String>,
    name: Option<String>,
    #[schemars(range(min = 1))]
    context: Option<u32>,
    output: Option<String>,
    efforts: Option<String>,
    extensions: Option<String>,
    fast_tier: Option<String>,
}

/// The input of `models remove`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ModelRemoveArgs {
    entry: String,
    /// The id of the model in the entry's list.
    model: String,
}

/// `list --json`.
#[derive(Serialize, JsonSchema)]
struct Listing {
    providers: Vec<ListedEntry>,
}

/// An entry as `list` lists it.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ListedEntry {
    id: String,
    /// The entry's name as Settings shows it.
    name: String,
    kind: CredentialKind,
    /// Whether its models can be used now, and what it lacks if not.
    availability: Availability,
    /// Whether the user turned it off in Settings > Providers.
    hidden: bool,
    models: Vec<ListedModel>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ListedModel {
    id: String,
    name: String,
    /// The context window in tokens; null when the catalog does not say.
    window: Option<u32>,
    efforts: Vec<String>,
    /// The effort a choice that names none takes: the first one listed.
    default_effort: Option<String>,
    /// Whether `off` turns its thinking off.
    can_turn_off: bool,
    tiers: Vec<String>,
    images: bool,
    /// Whether the calling agent runs on it.
    current: bool,
    /// Whether the user turned it off in Settings > Providers.
    hidden: bool,
}

/// `show --json`, and the answer of `add` and `edit`.
#[derive(Serialize, JsonSchema)]
struct ProviderAnswer {
    provider: ShownEntry,
}

/// Every setting of one entry; never its key.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ShownEntry {
    id: String,
    name: String,
    kind: CredentialKind,
    /// The vendor of a vendor entry; null for a compatible entry and a
    /// subscription.
    vendor: Option<String>,
    family: String,
    wire: Option<WireApi>,
    /// The configured endpoint; null for a vendor entry that uses its
    /// vendor's, and for a compatible entry not given one yet.
    endpoint: Option<String>,
    /// Whether a key is saved; null for a subscription.
    key_saved: Option<bool>,
    /// The labels of a subscription's accounts, for a user who configures
    /// the entries.
    accounts: Vec<String>,
    availability: Availability,
    hidden: bool,
    /// Where its models come from.
    models_from: ModelSource,
    models: Vec<ShownModel>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ModelSource {
    /// The entry's own list, which the `models` leaves change.
    List,
    /// Its vendor's list on models.dev.
    Vendor,
    /// The provider's own directory: a subscription's.
    Provider,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ShownModel {
    id: String,
    name: String,
    context_window: Option<u32>,
    output_limit: Option<u32>,
    efforts: Vec<String>,
    can_turn_off: bool,
    tiers: Vec<String>,
    /// The types the model reads natively: `[]` for none, null when unknown.
    accepted_extensions: Option<Vec<FileExtension>>,
    hidden: bool,
}

/// `remove --json`.
#[derive(Serialize, JsonSchema)]
struct RemovedAnswer {
    provider: RemovedEntry,
}

#[derive(Serialize, JsonSchema)]
struct RemovedEntry {
    id: String,
    name: String,
    removed: bool,
}

/// The answer of a `models` change: the model as the list holds it now,
/// or as it held it before it was removed.
#[derive(Serialize, JsonSchema)]
struct ModelAnswer {
    model: ConfiguredModel,
}

/// The `providers` group, whose handlers act in `shard`.
pub(crate) fn providers_group(shard: std::rc::Weak<Shard>) -> GroupBuilder {
    let models = GroupBuilder::new("models", MODELS_SUMMARY)
        .leaf(
            LeafBuilder::rpc("search", SEARCH_SUMMARY)
                .input::<SearchArgs>()
                .positionals(["query"])
                .json_output::<ModelSearchResults>()
                .bind(TypedRpc::new(verb(shard.clone(), search))),
        )
        .leaf(model_leaf::<ModelAddArgs>("add", MODEL_ADD_SUMMARY)
            .positionals(["entry", "model"])
            .bind(TypedRpc::new(verb(shard.clone(), add_model))))
        .leaf(model_leaf::<ModelEditArgs>("edit", MODEL_EDIT_SUMMARY)
            .describe("model", "The id of the model in the entry's list.")
            .positionals(["entry", "model"])
            .bind(TypedRpc::new(verb(shard.clone(), edit_model))))
        .leaf(
            LeafBuilder::rpc("remove", MODEL_REMOVE_SUMMARY)
                .input::<ModelRemoveArgs>()
                .describe("entry", ENTRY_ARG)
                .positionals(["entry", "model"])
                .json_output::<ModelAnswer>()
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard.clone(), remove_model))),
        );
    GroupBuilder::new("providers", SUMMARY)
        .index_entry(ENTRY)
        .permission(MANAGE, MANAGE_ACTION, MANAGE_DESCRIPTION)
        .leaf(
            LeafBuilder::rpc("list", LIST_SUMMARY)
                .input::<ListArgs>()
                .describe("all", ALL)
                .json_output::<Listing>()
                .bind(TypedRpc::new(verb(shard.clone(), list))),
        )
        .leaf(
            LeafBuilder::rpc("show", SHOW_SUMMARY)
                .input::<EntryArgs>()
                .describe("entry", ENTRY_ARG)
                .positionals(["entry"])
                .json_output::<ProviderAnswer>()
                .bind(TypedRpc::new(verb(shard.clone(), show))),
        )
        .leaf(
            LeafBuilder::rpc("add", ADD_SUMMARY)
                .input::<AddArgs>()
                .describe("vendor", VENDOR)
                .describe("family", FAMILY)
                .describe("wire", WIRE)
                .describe("base-url", BASE_URL)
                .describe("name", NAME)
                .describe("api-key-stdin", API_KEY_STDIN)
                .describe("api-key", API_KEY)
                .stdin_field("api-key")
                .stdin_with(["api-key-stdin"])
                .json_output::<ProviderAnswer>()
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard.clone(), add))),
        )
        .leaf(
            LeafBuilder::rpc("edit", EDIT_SUMMARY)
                .input::<EditArgs>()
                .describe("entry", ENTRY_ARG)
                .describe("name", NAME)
                .describe("base-url", BASE_URL)
                .describe("api-key-stdin", API_KEY_STDIN)
                .describe("api-key", API_KEY)
                .positionals(["entry"])
                .stdin_field("api-key")
                .stdin_with(["api-key-stdin"])
                .json_output::<ProviderAnswer>()
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard.clone(), edit))),
        )
        .leaf(
            LeafBuilder::rpc("remove", REMOVE_SUMMARY)
                .input::<EntryArgs>()
                .describe("entry", ENTRY_ARG)
                .positionals(["entry"])
                .json_output::<RemovedAnswer>()
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard, remove))),
        )
        .group(models)
}

/// A `models` leaf that sets a model's fields.
fn model_leaf<A: JsonSchema>(name: &str, summary: &str) -> LeafBuilder {
    LeafBuilder::rpc(name, summary)
        .input::<A>()
        .describe("entry", ENTRY_ARG)
        .describe("id", MODEL_ID)
        .describe("name", MODEL_NAME)
        .describe("context", CONTEXT)
        .describe("output", OUTPUT)
        .describe("efforts", EFFORTS)
        .describe("extensions", EXTENSIONS)
        .describe("fast-tier", FAST_TIER)
        .json_output::<ModelAnswer>()
        .permission(MANAGE)
}

fn failed(error: impl ToString) -> RpcError {
    RpcError::Failed(error.to_string())
}

async fn out(port: &RpcPort, text: String) -> Result<u8, RpcError> {
    port.stdout(text.into_bytes()).await?;
    Ok(0)
}

async fn json(port: &RpcPort, value: &impl Serialize) -> Result<u8, RpcError> {
    let text = serde_json::to_string(value).expect("a command's output serializes");
    out(port, format!("{text}\n")).await
}

async fn list(shard: Rc<Shard>, call: Call<ListArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let all = call.args.all == Some(true);
    let scope = shard.scope().await.map_err(failed)?;
    let current = caller_model(&shard, &call);
    let providers: Vec<ListedEntry> = scope
        .entries
        .iter()
        .filter_map(|entry| {
            let hidden = scope.preferences.hides_provider(&entry.id);
            if hidden && !all {
                return None;
            }
            let catalog = scope.catalog_of(entry)?;
            let models = catalog
                .models
                .iter()
                .filter_map(|model| {
                    let hidden = scope.preferences.hides_model(&entry.id, &model.model.id);
                    if hidden && !all {
                        return None;
                    }
                    let efforts = model.model.efforts();
                    Some(ListedModel {
                        id: model.model.id.clone(),
                        name: model.model.display_name.clone(),
                        window: model.model.context_window,
                        default_effort: efforts.first().cloned(),
                        efforts,
                        can_turn_off: model.model.can_turn_thinking_off(),
                        tiers: tiers(&model.model.service_tiers),
                        images: model_reads_images(&model.selection.model),
                        current: current.as_ref().is_some_and(|current| {
                            current.provider_id == entry.id.as_str()
                                && current.model.id == model.model.id
                        }),
                        hidden,
                    })
                })
                .collect();
            Some(ListedEntry {
                id: entry.id.as_str().to_owned(),
                name: entry.label.clone(),
                kind: entry.kind(),
                availability: catalog.availability.clone(),
                hidden,
                models,
            })
        })
        .collect();
    let listing = Listing { providers };
    if call.invocation.json {
        return json(&port, &listing).await;
    }
    out(&port, listing.render()).await
}

async fn show(shard: Rc<Shard>, call: Call<EntryArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let user = user(&shard).await?;
    let scope = shard.scope().await.map_err(failed)?;
    let entry = named(&scope.entries, &call.args.entry).map_err(failed)?;
    let shown = shown(&shard, &user, &scope, entry).await?;
    answer(&port, call.invocation.json, shown, None).await
}

async fn add(shard: Rc<Shard>, call: Call<AddArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let args = call.args;
    let user = user(&shard).await?;
    let api_key = key(args.api_key_stdin, args.api_key)?;
    let label = args.name.map(Trimmed::from);
    let base_url = args.base_url.map(endpoint).transpose()?;
    let wire_api = args.wire.map(wire).transpose()?;
    let body = match (args.vendor, args.family) {
        (Some(vendor_id), None) => {
            if wire_api.is_some() || base_url.is_some() {
                return Err(failed(
                    "--wire and --base-url are the vendor's; give them only with --family",
                ));
            }
            CreateProvider::Vendor {
                vendor_id,
                label,
                api_key,
                models: None,
            }
        }
        // An endpoint of the user's makes a compatible entry; without one
        // the entry is the family's own vendor's.
        (None, Some(provider_type)) if base_url.is_some() => CreateProvider::Compatible {
            provider_type: Some(provider_type),
            wire_api,
            label,
            api_key,
            base_url,
            models: None,
        },
        (None, Some(provider_type)) => CreateProvider::Family {
            provider_type,
            wire_api,
            label,
            api_key,
            models: None,
        },
        _ => {
            return Err(failed(
                "give --vendor, a models.dev vendor, or --family, with --base-url for a compatible API",
            ));
        }
    };
    body.validate().map_err(|report| failed(problems(&report)))?;
    let entry = shard
        .services()
        .create_provider(&user, body)
        .await
        .map_err(failed)?;
    let scope = shard.scope().await.map_err(failed)?;
    let entry = named(&scope.entries, entry.id.as_str()).map_err(failed)?;
    let shown = shown(&shard, &user, &scope, entry).await?;
    let note = format!("Added {} ({}).", shown.name, shown.id);
    answer(&port, call.invocation.json, shown, Some(note)).await
}

async fn edit(shard: Rc<Shard>, call: Call<EditArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let args = call.args;
    let user = user(&shard).await?;
    let patch = ProviderPatch {
        label: args.name.map(Trimmed::from),
        api_key: key(args.api_key_stdin, args.api_key)?.map(Some),
        base_url: args.base_url.map(endpoint).transpose()?.map(Some),
        ..ProviderPatch::default()
    };
    if patch.label.is_none() && patch.api_key.is_none() && patch.base_url.is_none() {
        return Err(failed(
            "nothing to change: give --name, --base-url or --api-key-stdin",
        ));
    }
    patch.validate().map_err(|report| failed(problems(&report)))?;
    let entries = entries(&shard).await.map_err(failed)?;
    let id = named(&entries, &args.entry).map_err(failed)?.id.clone();
    shard
        .services()
        .update_provider(&user, id.as_str(), patch)
        .await
        .map_err(failed)?;
    let scope = shard.scope().await.map_err(failed)?;
    let entry = named(&scope.entries, id.as_str()).map_err(failed)?;
    let shown = shown(&shard, &user, &scope, entry).await?;
    let note = format!("Changed {} ({}).", shown.name, shown.id);
    answer(&port, call.invocation.json, shown, Some(note)).await
}

async fn remove(shard: Rc<Shard>, call: Call<EntryArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let user = user(&shard).await?;
    let entries = entries(&shard).await.map_err(failed)?;
    let id = named(&entries, &call.args.entry).map_err(failed)?.id.clone();
    let removed = shard
        .services()
        .delete_provider(&user, id.as_str())
        .await
        .map_err(failed)?;
    let removed = RemovedEntry {
        id: removed.id.as_str().to_owned(),
        name: removed.label,
        removed: true,
    };
    if call.invocation.json {
        return json(&port, &RemovedAnswer { provider: removed }).await;
    }
    out(&port, format!("Removed {} ({}).\n", removed.name, removed.id)).await
}

async fn search(shard: Rc<Shard>, call: Call<SearchArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let query = call.args.query.join(" ");
    let results = shard
        .services()
        .search_models(shard.user(), &query, None)
        .await
        .map_err(failed)?;
    let found = ModelSearchResults { results };
    if call.invocation.json {
        return json(&port, &found).await;
    }
    if found.results.is_empty() {
        return out(&port, format!("No model on models.dev matches \"{query}\".\n")).await;
    }
    let rows: Vec<Vec<String>> = found.results.iter().map(match_row).collect();
    out(&port, columns(&rows)).await
}

async fn add_model(
    shard: Rc<Shard>,
    call: Call<ModelAddArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let args = call.args;
    let user = user(&shard).await?;
    let scope = shard.scope().await.map_err(failed)?;
    let entry = named(&scope.entries, &args.entry).map_err(failed)?;
    // An entry that follows its vendor's list starts its own from the
    // models the vendor offers now, so adding one removes none.
    let (mut models, started) = match list_of(entry)? {
        Some(models) => (models, false),
        None => (vendor_list(&scope, entry), true),
    };
    let found = match &args.model {
        Some(reference) => Some(
            shard
                .services()
                .models_dev_model(reference)
                .await
                .map_err(failed)?
                .ok_or_else(|| {
                    failed(format!(
                        "models.dev lists no model \"{reference}\"; `demi providers models search` finds one"
                    ))
                })?,
        ),
        None => None,
    };
    let mut model = new_model(found.as_ref(), args.fields)?;
    if started {
        // The vendor's own copy of the model gives way to the one added.
        models.retain(|listed| listed.id != model.id);
    }
    // A taken id takes the first free suffix (`models.md` § Adding a model
    // from models.dev).
    if models.iter().any(|listed| listed.id == model.id) {
        let id = (2..)
            .map(|number| format!("{}-{number}", model.id.as_str()))
            .find(|id| models.iter().all(|listed| listed.id.as_str() != id))
            .expect("a list holds finitely many models");
        model.id = Trimmed::from(id);
    }
    models.push(model.clone());
    let count = models.len();
    write_list(&shard, &user, entry, models).await?;
    let mut done = format!("Added to {}", entry.label);
    if started {
        done = format!(
            "{} now keeps its own list of {count} models; it no longer follows models.dev.\n{done}",
            entry.label
        );
    }
    model_answer(&port, call.invocation.json, model, &done).await
}

/// The models the entry's vendor offers now, as a configured list holds
/// them.
fn vendor_list(scope: &Scope, entry: &ProviderEntry) -> Vec<ConfiguredModel> {
    let catalog = scope.catalog_of(entry).map(|catalog| &catalog.models[..]).unwrap_or_default();
    catalog
        .iter()
        .map(|model| {
            let model = &model.model;
            ConfiguredModel {
                id: Trimmed::from(model.id.clone()),
                display_name: Trimmed::from(model.display_name.clone()),
                context_window: model.context_window,
                output_limit: model.output_limit,
                thinking_efforts: model.efforts(),
                accepted_extensions: model.accepted_extensions.clone(),
                fast_tier: model
                    .service_tiers
                    .iter()
                    .find(|tier| tier.fast)
                    .map(|tier| tier.id.clone()),
            }
        })
        .collect()
}

async fn edit_model(
    shard: Rc<Shard>,
    call: Call<ModelEditArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let args = call.args;
    let user = user(&shard).await?;
    let entries = entries(&shard).await.map_err(failed)?;
    let entry = named(&entries, &args.entry).map_err(failed)?;
    let mut models = owned_list(entry)?;
    let index = listed(entry, &models, &args.model)?;
    let fields = args.fields;
    let model = &mut models[index];
    if let Some(id) = fields.id {
        model.id = Trimmed::from(id);
    }
    if let Some(name) = fields.name {
        model.display_name = Trimmed::from(name);
    }
    if let Some(context) = fields.context {
        model.context_window = Some(context);
    }
    if let Some(output) = fields.output {
        model.output_limit = output_limit(&output)?;
    }
    if let Some(efforts) = fields.efforts {
        model.thinking_efforts = efforts_of(&efforts);
    }
    if let Some(extensions) = fields.extensions {
        model.accepted_extensions = extensions_of(&extensions)?;
    }
    if let Some(tier) = fields.fast_tier {
        model.fast_tier = (tier != NONE).then_some(tier);
    }
    let model = model.clone();
    write_list(&shard, &user, entry, models).await?;
    model_answer(&port, call.invocation.json, model, &format!("Changed in {}", entry.label)).await
}

async fn remove_model(
    shard: Rc<Shard>,
    call: Call<ModelRemoveArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let args = call.args;
    let user = user(&shard).await?;
    let entries = entries(&shard).await.map_err(failed)?;
    let entry = named(&entries, &args.entry).map_err(failed)?;
    let mut models = owned_list(entry)?;
    let index = listed(entry, &models, &args.model)?;
    let model = models.remove(index);
    write_list(&shard, &user, entry, models).await?;
    model_answer(&port, call.invocation.json, model, &format!("Removed from {}", entry.label)).await
}

/// The word that empties a field an option sets.
const NONE: &str = "none";

/// The user the shard serves, with the role that decides whether they
/// configure the scope's entries.
async fn user(shard: &Shard) -> Result<UserDto, RpcError> {
    shard
        .services()
        .control
        .account(shard.user().clone())
        .await
        .map_err(failed)?
        .map(|account| account.user)
        .ok_or_else(|| failed("the user's account is gone"))
}

/// The model the calling agent runs on, while its session is live.
fn caller_model<A>(shard: &Shard, call: &Call<A>) -> Option<ModelSelection> {
    let conversation = conversation_of(&call.invocation).ok()?;
    let job = call.invocation.caller.as_ref()?;
    let node = shard.agent().node(&root_of(&conversation), &job.node)?;
    Some(node.session().model())
}

/// The entry `named` names among `entries`: by its id, else by its name,
/// which must name one alone.
pub(crate) fn named<'a>(
    entries: &'a [ProviderEntry],
    named: &str,
) -> Result<&'a ProviderEntry, String> {
    if let Some(entry) = entries.iter().find(|entry| entry.id.as_str() == named) {
        return Ok(entry);
    }
    let matching: Vec<&ProviderEntry> = entries
        .iter()
        .filter(|entry| entry.label == named)
        .collect();
    match matching.as_slice() {
        [entry] => Ok(entry),
        [] => Err(format!(
            "no provider entry is named or has the id \"{named}\"; `demi providers list` lists them"
        )),
        several => {
            let ids: Vec<&str> = several.iter().map(|entry| entry.id.as_str()).collect();
            Err(format!(
                "several provider entries are named \"{named}\"; name one by its id: {}",
                ids.join(", ")
            ))
        }
    }
}

/// The key `--api-key-stdin` read, without the one newline that ends its
/// line; none without the flag.
fn key(flag: Option<bool>, stdin: Option<String>) -> Result<Option<String>, RpcError> {
    if flag != Some(true) {
        return Ok(None);
    }
    // The settings page's path checks what is left, with its words: one
    // line, not empty.
    let mut text = stdin.unwrap_or_default();
    if text.ends_with('\n') {
        text.pop();
        if text.ends_with('\r') {
            text.pop();
        }
    }
    Ok(Some(text))
}

/// `--wire` as a wire, refused in the words of a refused body.
fn wire(text: String) -> Result<WireApi, RpcError> {
    serde_json::from_value(serde_json::Value::String(text))
        .map_err(|error| failed(format!("wireApi: {error}")))
}

/// `--base-url` as an endpoint, refused in the words of a refused body.
fn endpoint(text: String) -> Result<EndpointUrl, RpcError> {
    EndpointUrl::try_from(text).map_err(|error| failed(format!("baseUrl: {error}")))
}

/// The entry's own model list, which a subscription entry has none of;
/// none for an entry whose models come from elsewhere.
fn list_of(entry: &ProviderEntry) -> Result<Option<Vec<ConfiguredModel>>, RpcError> {
    match &entry.credential {
        EntryCredential::ApiKey(config) => Ok(config.models.clone().map(|models| models.0)),
        EntryCredential::Subscription { .. } => {
            Err(failed(ProviderRefusal::OwnModels(entry.label.clone())))
        }
    }
}

/// The entry's own model list, which `models edit` and `remove` change.
fn owned_list(entry: &ProviderEntry) -> Result<Vec<ConfiguredModel>, RpcError> {
    list_of(entry)?.ok_or_else(|| {
        failed(format!(
            "{} has no model list of its own: its models come from its vendor or its provider; `demi providers models add` starts one",
            entry.label
        ))
    })
}

/// Where the model `id` is in `models`.
fn listed(entry: &ProviderEntry, models: &[ConfiguredModel], id: &str) -> Result<usize, RpcError> {
    models
        .iter()
        .position(|model| model.id.as_str() == id)
        .ok_or_else(|| {
            failed(format!(
                "{} has no model \"{id}\" in its list; `demi providers show {}` shows it",
                entry.label, entry.id
            ))
        })
}

/// Saves `models` as the entry's list through the settings page's path,
/// checked by the page's rules.
async fn write_list(
    shard: &Shard,
    user: &UserDto,
    entry: &ProviderEntry,
    models: Vec<ConfiguredModel>,
) -> Result<(), RpcError> {
    let patch = ProviderPatch {
        models: Some(Some(ConfiguredModels(models))),
        ..ProviderPatch::default()
    };
    patch.validate().map_err(|report| failed(problems(&report)))?;
    shard
        .services()
        .update_provider(user, entry.id.as_str(), patch)
        .await
        .map_err(failed)?;
    Ok(())
}

/// A new model: `found`'s facts, with what `fields` set over them.
fn new_model(found: Option<&ModelMatch>, fields: ModelFields) -> Result<ConfiguredModel, RpcError> {
    let id = fields
        .id
        .or_else(|| found.map(|found| found.id.clone()))
        .ok_or_else(|| failed("id: give the model's id with --id, or a search match"))?;
    let context_window = fields
        .context
        .or_else(|| found.and_then(|found| found.context_window));
    let output_limit = match fields.output {
        Some(output) => output_limit(&output)?,
        None => found.and_then(|found| found.output_limit),
    };
    let thinking_efforts = match fields.efforts {
        Some(efforts) => efforts_of(&efforts),
        None => found.map(|found| found.thinking_efforts.clone()).unwrap_or_default(),
    };
    let accepted_extensions = match fields.extensions {
        Some(extensions) => extensions_of(&extensions)?,
        None => found.and_then(|found| found.accepted_extensions.clone()),
    };
    let display_name = fields
        .name
        .or_else(|| found.map(|found| found.display_name.clone()))
        .unwrap_or_else(|| id.clone());
    Ok(ConfiguredModel {
        id: Trimmed::from(id),
        display_name: Trimmed::from(display_name),
        context_window,
        output_limit,
        thinking_efforts,
        accepted_extensions,
        fast_tier: fields.fast_tier.filter(|tier| tier != NONE),
    })
}

fn output_limit(text: &str) -> Result<Option<u32>, RpcError> {
    if text == NONE {
        return Ok(None);
    }
    text.parse::<u32>()
        .map(Some)
        .map_err(|_| failed(format!("outputLimit: \"{text}\" is not a whole number of tokens")))
}

fn efforts_of(text: &str) -> Vec<String> {
    if text == NONE {
        return Vec::new();
    }
    text.split(',')
        .map(str::trim)
        .filter(|effort| !effort.is_empty())
        .map(str::to_owned)
        .collect()
}

fn extensions_of(text: &str) -> Result<Option<Vec<FileExtension>>, RpcError> {
    match text {
        NONE => Ok(Some(Vec::new())),
        "unknown" => Ok(None),
        _ => text
            .split(',')
            .map(|extension| extension.trim().trim_start_matches('.'))
            .filter(|extension| !extension.is_empty())
            .map(|extension| {
                extension
                    .parse::<FileExtension>()
                    .map_err(|error| failed(format!("acceptedExtensions: {error}")))
            })
            .collect::<Result<Vec<_>, _>>()
            .map(Some),
    }
}

async fn model_answer(
    port: &RpcPort,
    json_output: bool,
    model: ConfiguredModel,
    done: &str,
) -> Result<u8, RpcError> {
    if json_output {
        return json(port, &ModelAnswer { model }).await;
    }
    let row = configured_row(&model);
    out(port, format!("{done}: {}\n", row.join("  "))).await
}

/// The entry as `show` and a change answer it.
async fn shown(
    shard: &Shard,
    user: &UserDto,
    scope: &Scope,
    entry: &ProviderEntry,
) -> Result<ShownEntry, RpcError> {
    let services = shard.services();
    let accounts = match entry.credential {
        EntryCredential::Subscription { .. } if services.vault.configures(user) => services
            .vault
            .accounts(entry.id.clone())
            .await
            .map_err(failed)?
            .into_iter()
            .map(|account| account.label)
            .collect(),
        _ => Vec::new(),
    };
    let (config, models_from) = match &entry.credential {
        // A compatible entry lists only its own models, none at first.
        EntryCredential::ApiKey(config) => (
            Some(config),
            if config.models.is_some() || config.is_compatible() {
                ModelSource::List
            } else {
                ModelSource::Vendor
            },
        ),
        EntryCredential::Subscription { .. } => (None, ModelSource::Provider),
    };
    let catalog = scope.catalog_of(entry);
    let models = catalog
        .map(|catalog| {
            catalog
                .models
                .iter()
                .map(|model| ShownModel {
                    id: model.model.id.clone(),
                    name: model.model.display_name.clone(),
                    context_window: model.model.context_window,
                    output_limit: model.model.output_limit,
                    efforts: model.model.efforts(),
                    can_turn_off: model.model.can_turn_thinking_off(),
                    tiers: tiers(&model.model.service_tiers),
                    accepted_extensions: model.model.accepted_extensions.clone(),
                    hidden: scope.preferences.hides_model(&entry.id, &model.model.id),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(ShownEntry {
        id: entry.id.as_str().to_owned(),
        name: entry.label.clone(),
        kind: entry.kind(),
        vendor: config.and_then(|config| config.vendor_id.clone()),
        family: entry.family.clone(),
        wire: config.and_then(|config| config.wire_api),
        endpoint: config.and_then(|config| config.base_url.as_ref().map(|url| url.url().to_string())),
        key_saved: config.map(|config| config.api_key.is_some()),
        accounts,
        availability: catalog.map_or_else(
            || Availability::setup(entry.missing()),
            |catalog| catalog.availability.clone(),
        ),
        hidden: scope.preferences.hides_provider(&entry.id),
        models_from,
        models,
    })
}

async fn answer(
    port: &RpcPort,
    json_output: bool,
    shown: ShownEntry,
    note: Option<String>,
) -> Result<u8, RpcError> {
    if json_output {
        return json(port, &ProviderAnswer { provider: shown }).await;
    }
    let mut text = note.map(|note| format!("{note}\n")).unwrap_or_default();
    text.push_str(&shown.render());
    out(port, text).await
}

fn tiers(tiers: &[demi_shared_types::ServiceTier]) -> Vec<String> {
    tiers.iter().map(|tier| tier.id.clone()).collect()
}

/// A token count as a menu shows it: `1M`, `128K`, `1.2M`.
fn tokens(count: u32) -> String {
    let scaled = |value: f64, unit: &str| {
        let text = format!("{value:.1}");
        format!("{}{unit}", text.strip_suffix(".0").unwrap_or(&text))
    };
    match count {
        1_000_000.. => scaled(f64::from(count) / 1_000_000.0, "M"),
        1_000.. => scaled(f64::from(count) / 1_000.0, "K"),
        _ => count.to_string(),
    }
}

/// Efforts as a list shows them: the default marked `*`, and `off` last
/// when thinking can be turned off.
fn efforts_text(efforts: &[String], can_turn_off: bool) -> String {
    let mut words: Vec<String> = efforts
        .iter()
        .enumerate()
        .map(|(index, effort)| {
            if index == 0 {
                format!("{effort}*")
            } else {
                effort.clone()
            }
        })
        .collect();
    if can_turn_off {
        words.push(EFFORT_OFF.to_owned());
    }
    words.join(" ")
}

/// The files a model reads, as a row shows them.
fn files_text(extensions: Option<&[FileExtension]>) -> String {
    match extensions {
        None => "files unknown".to_owned(),
        Some([]) => "no files".to_owned(),
        Some(extensions) => extensions
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" "),
    }
}

/// What an entry's state says: Ready, what it still needs, or why its
/// models cannot be used.
fn state(availability: &Availability) -> String {
    match availability {
        Availability::Available {} => "Ready".to_owned(),
        Availability::Unavailable {
            missing: Some(missing),
            ..
        } => {
            let parts: Vec<&str> = missing
                .iter()
                .map(|part| match part {
                    SetupPart::ApiKey => "an API key",
                    SetupPart::Endpoint => "an endpoint",
                    SetupPart::Models => "a model",
                    SetupPart::Account => "a signed-in account",
                })
                .collect();
            format!("Needs {}", and_list(&parts))
        }
        Availability::Unavailable { message, .. } => format!("Unavailable: {message}"),
    }
}

/// `a`, `a and b`, `a, b and c`.
fn and_list(parts: &[&str]) -> String {
    match parts {
        [] => String::new(),
        [one] => (*one).to_owned(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn kind_text(kind: CredentialKind) -> &'static str {
    match kind {
        CredentialKind::Subscription => "subscription",
        CredentialKind::ApiKey => "API key",
    }
}

/// Rows in columns two spaces apart, a column no row fills left out.
fn columns(rows: &[Vec<String>]) -> String {
    let count = rows.iter().map(Vec::len).max().unwrap_or(0);
    let mut widths = vec![0; count];
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let mut text = String::new();
    for row in rows {
        let line: Vec<String> = row
            .iter()
            .zip(&widths)
            .filter(|(_, width)| **width > 0)
            .map(|(cell, width)| format!("{cell:width$}"))
            .collect();
        text.push_str(line.join("  ").trim_end());
        text.push('\n');
    }
    text
}

/// A search match as `models search` lists it.
fn match_row(found: &ModelMatch) -> Vec<String> {
    vec![
        found.reference.clone(),
        found.display_name.clone(),
        found.context_window.map_or_else(|| "-".to_owned(), tokens),
        found
            .output_limit
            .map_or_else(|| "-".to_owned(), |limit| format!("out {}", tokens(limit))),
        efforts_text(&found.thinking_efforts, false),
        files_text(found.accepted_extensions.as_deref()),
    ]
}

/// A configured model as a change answers it.
fn configured_row(model: &ConfiguredModel) -> Vec<String> {
    let mut row = vec![
        model.id.as_str().to_owned(),
        model.display_name.as_str().to_owned(),
        model.context_window.map_or_else(|| "-".to_owned(), tokens),
        model
            .output_limit
            .map_or_else(|| "-".to_owned(), |limit| format!("out {}", tokens(limit))),
        efforts_text(&model.thinking_efforts, false),
        files_text(model.accepted_extensions.as_deref()),
    ];
    if let Some(tier) = &model.fast_tier {
        row.push(format!("fast {tier}"));
    }
    row.retain(|cell| !cell.is_empty());
    row
}

impl Listing {
    /// The text `list` prints: each entry's name, kind and state, then a row
    /// per model in columns, the calling agent's marked with `*`, as is
    /// each default effort, and the mark's key last.
    fn render(&self) -> String {
        if self.providers.is_empty() {
            return "The user has no provider entry; `demi providers add` adds one, or the user adds one in Settings > Providers.\n".to_owned();
        }
        let rows: Vec<Vec<String>> = self
            .providers
            .iter()
            .flat_map(|entry| &entry.models)
            .map(|model| {
                vec![
                    format!("{} {}", if model.current { '*' } else { ' ' }, model.id),
                    model.name.clone(),
                    model.window.map_or_else(|| "-".to_owned(), tokens),
                    efforts_text(&model.efforts, model.can_turn_off),
                    model.tiers.join(" "),
                    if model.images { "images" } else { "-" }.to_owned(),
                    if model.hidden { "turned off" } else { "" }.to_owned(),
                ]
            })
            .collect();
        let mut lines = columns(&rows).lines().map(str::to_owned).collect::<Vec<_>>().into_iter();
        let mut text = String::new();
        for entry in &self.providers {
            let hidden = if entry.hidden { " · turned off" } else { "" };
            text.push_str(&format!(
                "{} · {} · {}{hidden}\n",
                entry.name,
                kind_text(entry.kind),
                state(&entry.availability)
            ));
            for line in lines.by_ref().take(entry.models.len()) {
                text.push_str(&line);
                text.push('\n');
            }
        }
        text.push_str("* the model this agent runs on; an effort with * is the model's default\n");
        text
    }
}

impl ShownEntry {
    /// The text `show` prints: a setting per line, then a row per model.
    fn render(&self) -> String {
        let mut fields = vec![
            ("name", self.name.clone()),
            ("id", self.id.clone()),
            ("kind", kind_text(self.kind).to_owned()),
        ];
        if let Some(vendor) = &self.vendor {
            fields.push(("vendor", vendor.clone()));
        }
        fields.push(("family", self.family.clone()));
        if let Some(wire) = self.wire {
            fields.push(("wire", wire.to_string()));
        }
        // A compatible entry names no vendor, and has no endpoint until the
        // user gives one.
        if self.kind == CredentialKind::ApiKey {
            let none = match self.vendor {
                Some(_) => "the vendor's",
                None => "not set",
            };
            fields.push((
                "endpoint",
                self.endpoint.clone().unwrap_or_else(|| none.to_owned()),
            ));
        }
        if let Some(saved) = self.key_saved {
            fields.push(("key", if saved { "saved" } else { "not saved" }.to_owned()));
        }
        if self.kind == CredentialKind::Subscription && !self.accounts.is_empty() {
            fields.push(("accounts", self.accounts.join(", ")));
        }
        fields.push(("state", state(&self.availability)));
        if self.hidden {
            fields.push(("menu", "turned off in Settings > Providers".to_owned()));
        }
        fields.push((
            "models",
            match self.models_from {
                ModelSource::List => "its own list",
                ModelSource::Vendor => "its vendor's list on models.dev",
                ModelSource::Provider => "the provider's own list",
            }
            .to_owned(),
        ));
        let mut text: String = fields
            .iter()
            .map(|(field, value)| format!("{field:10}{value}\n"))
            .collect();
        let rows: Vec<Vec<String>> = self
            .models
            .iter()
            .map(|model| {
                vec![
                    format!("  {}", model.id),
                    model.name.clone(),
                    model.context_window.map_or_else(|| "-".to_owned(), tokens),
                    model
                        .output_limit
                        .map_or_else(|| "-".to_owned(), |limit| format!("out {}", tokens(limit))),
                    efforts_text(&model.efforts, model.can_turn_off),
                    model.tiers.join(" "),
                    files_text(model.accepted_extensions.as_deref()),
                    if model.hidden { "turned off" } else { "" }.to_owned(),
                ]
            })
            .collect();
        text.push_str(&columns(&rows));
        text
    }
}
