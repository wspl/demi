//! The product's `demi providers` group.
//!
//! The agent lists and shows the user's provider entries with their models,
//! adds, edits and removes entries, turns them and their models off and on in
//! the model menu, chooses and orders a subscription's accounts, and searches
//! models.dev to add, edit and remove the models of an entry's configured list.
//!
//! `edit` takes the entry `show --json` prints as a JSON merge patch, so every
//! setting Settings > Providers changes has one name here, the protocol's,
//! and a new setting needs no new option.
//!
//! A change goes through the settings page's path, with its checks and its words
//! (`provider_settings`), and needs the user's permission, the category
//! [`MANAGE`].
//!
//! No command takes a key as an argument or prints one. `add` and `edit` read a
//! key from stdin with `--api-key-stdin` alone, from a credential the agent
//! piped there, since a key typed into a conversation would sit in its
//! transcript and reach the model's vendor.

use demi_core_sync::Part;
use demi_core_providers::vault::accounts as vault_accounts;
use demi_core_providers::vault::entries::Vault;
use std::rc::Rc;

use demi_backend_host_access::host_commands::conversation_of;
use demi_backend_host_access::root_of;
use demi_provider_registry::entry::{EntryCredential, ProviderEntry};
use demi_host_interface::{Call, GroupBuilder, LeafBuilder, RpcError, RpcInvocation, RpcPort, TypedRpc};
use demi_agent_tools::EFFORT_OFF;
use demi_shared_types::{
    AccountInfo, ContextChoice, FileExtension, ModelSelection, ThinkingForm, WireApi, applied_context_limit, context_word, model_reads_images,
    output::{Cell, Fields, Table, note, warning},
};
use demi_web_api_protocol::ids::CredentialId;
use demi_web_api_protocol::auth::UserDto;
use demi_web_api_protocol::settings::{ContextLimitChange, HiddenModel, PreferencesPatch};
use demi_web_api_protocol::error::problems;
use demi_web_api_protocol::providers::{
    Availability, ConfiguredModel, ConfiguredModels, CreateProvider,
    CredentialKind, ModelKind, ModelMatch, ProviderDto, ProviderPatch, SetupPart,
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

const SUMMARY: &str = "The user's provider entries and their models: list and show them; add, edit and remove entries; turn an entry or a model off and on in the model menu; choose and order a subscription's accounts; and search models.dev to add models to an entry's list. A change needs the user's permission and reaches every conversation.";

/// The group's entry in the model's capability index.
const ENTRY: &str = "The user's model providers and their models.";

const LIST_SUMMARY: &str = "List each provider entry: its id, its name in Settings, its kind, its count of models and its status (Ready, or what it still needs); demi providers show lists an entry's models. --all adds the entries and models the user turned off. A read.";

const SHOW_SUMMARY: &str = "Show every setting of one entry, as Settings > Providers has it (never its key): with --json the entry `edit` changes, field by field, beside its status, whether it is hidden, its accounts by label and under catalog each model it offers with every field: context window and output limit in tokens, default effort and efforts (off when thinking can be turned off), service tiers, the files it reads, and whether you run on it. A read.";

const ADD_SUMMARY: &str = "Add a provider entry: --vendor, a models.dev vendor; --family alone, the family's own vendor; or --family with --wire and --base-url, a compatible API. Without --label it is named after its vendor or family, or Compatible API, with (2), (3) when that name is taken. A vendor's entry without a key joins the list incomplete until the user adds the key in Settings > Providers; a compatible API's key is optional. `edit` sets the rest. Needs the user's permission.";

const EDIT_SUMMARY: &str = "Change any setting of an entry that Settings > Providers changes, by a JSON merge patch on stdin of the entry `demi providers show --json` prints: a field given takes its value, a field left out stays, a list is replaced whole, and null removes a setting `show` can print as null, such as cacheTtl or throughDevice, or the key with apiKey. A field that holds the value shown changes nothing, and what `show` prints beside the entry (status, hidden, accounts, modelsFrom, catalog) is left as it is, so the printed entry, edited, may go back whole. --api-key-stdin reads the key alone instead. Needs the user's permission.";

const CONTEXT_SUMMARY: &str = "Set the context window Demi uses for one of an entry's models, in every conversation, as the model menu's Context row does: a size up to the model's window, such as 350K, 1M or 272000, which Demi compacts the conversation to fit, or full for the model's whole window, as its own size is. Needs the user's permission.";

const HIDE_SUMMARY: &str = "Turn an entry, or with --model one of its models, off in the user's model menu, as Settings > Providers' Show in Model Menu does. Needs the user's permission.";

const UNHIDE_SUMMARY: &str = "Turn an entry, or with --model one of its models, back on in the user's model menu. Needs the user's permission.";

const ACCOUNTS_SUMMARY: &str = "A subscription entry's accounts: list them, choose the one it infers with, and put them in the order In Order takes them. Signing in to an account is the user's, in Settings > Providers.";

const ACCOUNTS_LIST_SUMMARY: &str = "List a subscription entry's accounts in their order: each one's id, label and whether the entry infers with it now. A read.";

const ACCOUNTS_ACTIVATE_SUMMARY: &str = "Make one of a subscription entry's accounts the one it infers with. Needs the user's permission.";

const ACCOUNTS_ORDER_SUMMARY: &str = "Put a subscription entry's accounts in the order given, which In Order takes them in; name every account once. Needs the user's permission.";

const ORDER_SUMMARY: &str = "Put the entries in an order, the one Settings > Providers shows and generated images, videos and audio try them in, subscriptions before keys: the entries named come first, in the order given, and the others keep theirs after them. Needs the user's permission.";

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
const LABEL: &str = "The entry's name, 1 to 80 characters, as Settings shows it.";
const BASE_URL: &str = "The API's address, an http or https URL such as https://api.example.com/v1.";
const API_KEY_STDIN: &str = "Read the entry's key from stdin, one line; pipe it in, never type it into a command.";
const API_KEY: &str = "The key, read from stdin with --api-key-stdin.";
const PATCH: &str = "The changes, a JSON object such as {\"cacheTtl\": \"1h\", \"throughDevice\": null}; with --api-key-stdin, the key.";
const ENTRIES: &str = "The entries, by name or id, in the order to put them.";
const KIND: &str = "What the model makes: text, the model menu's; audio, for a compatible API whose audio shape is chosen, making what its shape makes, speech for openai-speech. Text without it.";
const MODEL_ID: &str = "The model's id as the entry's vendor names it.";
const MODEL_NAME: &str = "The model's name in the model menu; its id without one.";
const CONTEXT: &str = "The context window in tokens.";
const OUTPUT: &str = "The output limit in tokens, at most the context window, or none.";
const EFFORTS: &str = "The thinking efforts, comma-separated, the default first, such as low,medium,high; none for a model without thinking.";
const EXTENSIONS: &str = "The file types the model reads, comma-separated, such as png,jpg,pdf; none for no file, unknown when not known.";
const FAST_TIER: &str = "The service tier that is the model's Fast, such as priority, or none.";
const THINKING: &str = "A compatible API's model on the Messages API: how a thinking effort is sent, effort, as Anthropic's adaptive thinking, or budget, as a number of tokens. A vendor's model takes it from models.dev.";
const INTERLEAVED_THINKING: &str = "A compatible API's model on Chat Completions: whether its earlier thinking goes back with each tool call, as DeepSeek and Kimi need, on or off. A vendor's model takes it from models.dev.";

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
    label: Option<String>,
    api_key_stdin: Option<bool>,
    api_key: Option<String>,
}

/// The input of `edit`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct EditArgs {
    entry: String,
    api_key_stdin: Option<bool>,
    /// The JSON merge patch, or with `--api-key-stdin` the key, read from
    /// stdin.
    patch: Option<String>,
}

/// The input of `accounts activate`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AccountArgs {
    entry: String,
    /// The account's label or id, as `demi providers accounts list` shows them.
    account: String,
}

/// The input of `accounts order`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AccountOrderArgs {
    entry: String,
    /// The accounts, by label or id, in the order to put them.
    #[schemars(length(min = 1))]
    account: Vec<String>,
}

/// The input of `hide` and `unhide`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct HideArgs {
    entry: String,
    /// One of the entry's models, by id; the whole entry without it.
    model: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ContextArgs {
    entry: String,
    /// One of the entry's models, by id.
    model: String,
    /// `full`, or a size up to the model's window, such as `350K`, `1M` or
    /// `272000`; the window's own size is `full`.
    size: String,
}

/// `context --json`: the window Demi uses for the model now.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ModelContext {
    entry: String,
    model: String,
    /// The model's own window, in tokens.
    window: u32,
    /// The user's limit on it, in tokens; null for the whole window.
    limit: Option<u32>,
}

/// An account as `accounts list` lists it.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ListedAccount {
    id: String,
    label: String,
    /// Whether the entry infers with it now.
    active: bool,
}

/// The input of `order`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OrderArgs {
    #[schemars(length(min = 1))]
    entry: Vec<String>,
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
    kind: Option<ModelKind>,
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
    thinking: Option<ThinkingForm>,
    interleaved_thinking: Option<Switch>,
}

/// An option that turns something on or off.
#[derive(Deserialize, JsonSchema, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum Switch {
    On,
    Off,
}

impl Switch {
    fn on(self) -> bool {
        matches!(self, Self::On)
    }
}

/// The input of `models remove`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ModelRemoveArgs {
    entry: String,
    /// The id of the model in the entry's list.
    model: String,
}

/// An entry as `list` lists it.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ListedEntry {
    id: String,
    /// The entry's name as Settings shows it.
    label: String,
    kind: CredentialKind,
    /// Whether its models can be used now, and what it lacks if not.
    status: Availability,
    /// Whether the user turned it off in Settings > Providers.
    hidden: bool,
    models: Vec<ListedModel>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ListedModel {
    id: String,
    display_name: String,
    /// The context window in tokens; null when the catalog does not say.
    context_window: Option<u32>,
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

/// `show --json`, and the answer of `add` and `edit`: the entry as the API
/// answers it, which `edit` patches, with what is not the entry's beside
/// it. Never its key.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ShownEntry {
    #[serde(flatten)]
    entry: ProviderDto,
    /// Whether its models can be used now, and what it lacks if not.
    status: Availability,
    /// Whether the user turned it off in the model menu.
    hidden: bool,
    /// The labels of a subscription's accounts, in In Order's order, for a
    /// user who configures the entries.
    accounts: Vec<String>,
    /// Where the models it offers come from.
    models_from: ModelSource,
    /// The text models it offers now, which the model menu lists.
    catalog: Vec<ShownModel>,
    /// The name of the device its requests go out from, which the text
    /// says in place of its id; the id itself once the device is gone.
    #[serde(skip)]
    through_name: Option<String>,
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
    display_name: String,
    context_window: Option<u32>,
    output_limit: Option<u32>,
    efforts: Vec<String>,
    can_turn_off: bool,
    tiers: Vec<String>,
    /// The types the model reads natively: `[]` for none, null when unknown.
    accepted_extensions: Option<Vec<FileExtension>>,
    hidden: bool,
    /// Whether the calling agent runs on it.
    current: bool,
}

/// `remove --json`.
#[derive(Serialize, JsonSchema)]
struct RemovedEntry {
    id: String,
    label: String,
    removed: bool,
}

/// The `providers` group, whose handlers act in `shard`.
pub(crate) fn providers_group(shard: std::rc::Weak<Shard>) -> GroupBuilder {
    let models = GroupBuilder::new("models", MODELS_SUMMARY)
        .leaf(
            LeafBuilder::rpc("search", SEARCH_SUMMARY)
                .input::<SearchArgs>()
                .positionals(["query"])
                .json_output::<Vec<ModelMatch>>()
                .bind(TypedRpc::new(verb(shard.clone(), search))),
        )
        .leaf(model_leaf::<ModelAddArgs>("add", MODEL_ADD_SUMMARY)
            .describe("kind", KIND)
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
                .json_output::<ConfiguredModel>()
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
                .json_output::<Vec<ListedEntry>>()
                .bind(TypedRpc::new(verb(shard.clone(), list))),
        )
        .leaf(
            LeafBuilder::rpc("show", SHOW_SUMMARY)
                .input::<EntryArgs>()
                .describe("entry", ENTRY_ARG)
                .positionals(["entry"])
                .json_output::<ShownEntry>()
                .bind(TypedRpc::new(verb(shard.clone(), show))),
        )
        .leaf(
            LeafBuilder::rpc("add", ADD_SUMMARY)
                .input::<AddArgs>()
                .describe("vendor", VENDOR)
                .describe("family", FAMILY)
                .describe("wire", WIRE)
                .describe("base-url", BASE_URL)
                .describe("label", LABEL)
                .describe("api-key-stdin", API_KEY_STDIN)
                .describe("api-key", API_KEY)
                .stdin_field("api-key")
                .stdin_with(["api-key-stdin"])
                .json_output::<ShownEntry>()
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard.clone(), add))),
        )
        .leaf(
            LeafBuilder::rpc("edit", EDIT_SUMMARY)
                .input::<EditArgs>()
                .describe("entry", ENTRY_ARG)
                .describe("api-key-stdin", API_KEY_STDIN)
                .describe("patch", PATCH)
                .positionals(["entry"])
                .stdin_field("patch")
                .json_output::<ShownEntry>()
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard.clone(), edit))),
        )
        .leaf(
            LeafBuilder::rpc("context", CONTEXT_SUMMARY)
                .input::<ContextArgs>()
                .describe("entry", ENTRY_ARG)
                .positionals(["entry", "model", "size"])
                .json_output::<ModelContext>()
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard.clone(), set_context))),
        )
        .leaf(
            LeafBuilder::rpc("hide", HIDE_SUMMARY)
                .input::<HideArgs>()
                .describe("entry", ENTRY_ARG)
                .positionals(["entry"])
                .json_output::<ShownEntry>()
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard.clone(), hide))),
        )
        .leaf(
            LeafBuilder::rpc("unhide", UNHIDE_SUMMARY)
                .input::<HideArgs>()
                .describe("entry", ENTRY_ARG)
                .positionals(["entry"])
                .json_output::<ShownEntry>()
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard.clone(), unhide))),
        )
        .leaf(
            LeafBuilder::rpc("remove", REMOVE_SUMMARY)
                .input::<EntryArgs>()
                .describe("entry", ENTRY_ARG)
                .positionals(["entry"])
                .json_output::<RemovedEntry>()
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard.clone(), remove))),
        )
        .leaf(
            LeafBuilder::rpc("order", ORDER_SUMMARY)
                .input::<OrderArgs>()
                .describe("entry", ENTRIES)
                .positionals(["entry"])
                .json_output::<Vec<ListedEntry>>()
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard.clone(), order))),
        )
        .group(models)
        .group(
            GroupBuilder::new("accounts", ACCOUNTS_SUMMARY)
                .leaf(
                    LeafBuilder::rpc("list", ACCOUNTS_LIST_SUMMARY)
                        .input::<EntryArgs>()
                        .describe("entry", ENTRY_ARG)
                        .positionals(["entry"])
                        .json_output::<Vec<ListedAccount>>()
                        .bind(TypedRpc::new(verb(shard.clone(), list_accounts))),
                )
                .leaf(
                    LeafBuilder::rpc("activate", ACCOUNTS_ACTIVATE_SUMMARY)
                        .input::<AccountArgs>()
                        .describe("entry", ENTRY_ARG)
                        .positionals(["entry", "account"])
                        .json_output::<Vec<ListedAccount>>()
                        .permission(MANAGE)
                        .bind(TypedRpc::new(verb(shard.clone(), activate_account))),
                )
                .leaf(
                    LeafBuilder::rpc("order", ACCOUNTS_ORDER_SUMMARY)
                        .input::<AccountOrderArgs>()
                        .describe("entry", ENTRY_ARG)
                        .positionals(["entry", "account"])
                        .json_output::<Vec<ListedAccount>>()
                        .permission(MANAGE)
                        .bind(TypedRpc::new(verb(shard, order_accounts))),
                ),
        )
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
        .describe("thinking", THINKING)
        .describe("interleaved-thinking", INTERLEAVED_THINKING)
        .json_output::<ConfiguredModel>()
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
    let current = caller_model(&shard, &call.invocation);
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
                        display_name: model.model.display_name.clone(),
                        context_window: model.model.context_window,
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
                label: entry.label.clone(),
                kind: entry.kind(),
                status: catalog.availability.clone(),
                hidden,
                models,
            })
        })
        .collect();
    if call.invocation.json {
        return json(&port, &providers).await;
    }
    let mut table = Table::new(["id", "name", "kind", "models", "hidden", "status"]);
    for entry in &providers {
        table.row::<Cell>([
            entry.id.as_str().into(),
            entry.label.as_str().into(),
            kind_text(entry.kind).into(),
            entry.models.len().into(),
            entry.hidden.into(),
            state(&entry.status).into(),
        ]);
    }
    out(&port, table.render()).await
}

async fn show(shard: Rc<Shard>, call: Call<EntryArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let user = user(&shard).await?;
    let scope = shard.scope().await.map_err(failed)?;
    let entry = named(&scope.entries, &call.args.entry).map_err(failed)?;
    let current = caller_model(&shard, &call.invocation);
    let shown = shown(&shard, &user, &scope, entry, current.as_ref()).await?;
    answer(&port, call.invocation.json, shown).await
}

async fn add(shard: Rc<Shard>, call: Call<AddArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let args = call.args;
    let user = user(&shard).await?;
    let api_key = key(args.api_key_stdin, args.api_key)?;
    let label = args.label.map(Trimmed::from);
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
    let current = caller_model(&shard, &call.invocation);
    let shown = shown(&shard, &user, &scope, entry, current.as_ref()).await?;
    answer(&port, call.invocation.json, shown).await
}

async fn edit(shard: Rc<Shard>, call: Call<EditArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let args = call.args;
    let user = user(&shard).await?;
    let scope = shard.scope().await.map_err(failed)?;
    let entry = named(&scope.entries, &args.entry).map_err(failed)?;
    let id = entry.id.clone();
    let command = call.invocation.path.join(" ");
    let patch = if args.api_key_stdin == Some(true) {
        Some(ProviderPatch {
            api_key: Some(key(Some(true), args.patch)?),
            ..ProviderPatch::default()
        })
    } else {
        let current = caller_model(&shard, &call.invocation);
        let shown = shown(&shard, &user, &scope, entry, current.as_ref()).await?;
        let (patch, left) = entry_patch(&shown, args.patch.as_deref().unwrap_or_default())?;
        if !left.is_empty() {
            let what = format!(
                "{}: not the entry's, which edit leaves as they are; `demi providers hide`, `unhide` and `accounts` change what is the user's",
                left.join(", ")
            );
            port.stderr(warning(&command, &what)).await?;
        }
        patch
    };
    match patch {
        Some(patch) => {
            patch.validate().map_err(|report| failed(problems(&report)))?;
            shard
                .services()
                .update_provider(&user, id.as_str(), patch)
                .await
                .map_err(failed)?;
        }
        None => {
            let what = "no change: every setting holds the value given";
            port.stderr(note(&command, what)).await?;
        }
    }
    let scope = shard.scope().await.map_err(failed)?;
    let entry = named(&scope.entries, id.as_str()).map_err(failed)?;
    let current = caller_model(&shard, &call.invocation);
    let shown = shown(&shard, &user, &scope, entry, current.as_ref()).await?;
    answer(&port, call.invocation.json, shown).await
}

/// The patch that `text`, a JSON merge patch of the entry `shown` prints,
/// asks for (RFC 7396), none when it changes nothing, and the fields it
/// gave other values of what `show` prints beside the entry.
///
/// - A field that holds the value shown changes nothing and is dropped, in
///   an object as at the top, so the printed entry, edited, goes back as it
///   is.
/// - What is printed beside the entry, such as its status, is left out
///   whatever it holds, as Kubernetes leaves a resource's status out of an
///   update: it changes by itself between a `show` and an `edit`, and its
///   commands, not `edit`, change what of it is the user's.
/// - A field of the entry that no patch changes, such as its id, is
///   refused by its name when it holds another value, as one the patch
///   does not know is.
/// - The key is refused, which a transcript would keep; `null`, which
///   removes it as Settings does, is not.
fn entry_patch(shown: &ShownEntry, text: &str) -> Result<(Option<ProviderPatch>, Vec<String>), RpcError> {
    let patch: serde_json::Value = serde_json::from_str(text)
        .map_err(|error| failed(format!("stdin: give the changes as a JSON object, such as {{\"label\": \"Team gateway\"}}: {error}")))?;
    let serde_json::Value::Object(mut patch) = patch else {
        return Err(failed("stdin: give the changes as a JSON object, such as {\"label\": \"Team gateway\"}"));
    };
    if patch.get("apiKey").is_some_and(|key| !key.is_null()) {
        return Err(failed("apiKey: pipe the key in with --api-key-stdin, which keeps it out of the patch; null removes it"));
    }
    let entry = serde_json::to_value(&shown.entry).expect("an entry serializes");
    let shown = serde_json::to_value(shown).expect("a shown entry serializes");
    let mut left = Vec::new();
    patch.retain(|field, value| {
        let beside = entry.get(field).is_none() && shown.get(field).is_some();
        if beside && shown.get(field) != Some(value) {
            left.push(field.clone());
        }
        !beside
    });
    let changes = changed_fields(patch, &entry);
    if changes.is_empty() {
        return Ok((None, left));
    }
    let settings = schemars::schema_for!(ProviderPatch);
    let settings = settings.get("properties").and_then(|properties| properties.as_object());
    if let Some(field) = changes.keys().find(|field| !settings.is_some_and(|settings| settings.contains_key(*field))) {
        return Err(failed(if entry.get(field).is_some() {
            format!("{field}: the entry's own, which no edit changes")
        } else {
            format!("{field}: no such setting; `demi providers show --json` prints each")
        }));
    }
    let patch = serde_json::from_value(serde_json::Value::Object(changes)).map_err(|error| failed(format!("stdin: {error}")))?;
    Ok((Some(patch), left))
}

/// The fields of `patch` whose values differ from `shown`'s, an object's
/// own fields compared one by one.
fn changed_fields(
    patch: serde_json::Map<String, serde_json::Value>,
    shown: &serde_json::Value,
) -> serde_json::Map<String, serde_json::Value> {
    patch
        .into_iter()
        .filter_map(|(field, value)| {
            let was = shown.get(&field);
            match (value, was) {
                (value, Some(was)) if value == *was => None,
                (serde_json::Value::Object(fields), Some(was @ serde_json::Value::Object(_))) => {
                    let fields = changed_fields(fields, was);
                    (!fields.is_empty()).then_some((field, serde_json::Value::Object(fields)))
                }
                (value, _) => Some((field, value)),
            }
        })
        .collect()
}

/// Stores the user's limit on a model, or removes it, as the model menu's
/// Context row does: the same patch of the preferences, read and written
/// in one transaction.
async fn set_context(shard: Rc<Shard>, call: Call<ContextArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let args = call.args;
    let user = user(&shard).await?;
    let scope = shard.scope().await.map_err(failed)?;
    let entry = named(&scope.entries, &args.entry).map_err(failed)?;
    let Some(model) = scope
        .catalog_of(entry)
        .and_then(|catalog| catalog.models.iter().find(|offered| offered.model.id == args.model))
    else {
        return Err(failed(format!(
            "{} offers no model {}; `demi providers show {}` lists its models",
            entry.label,
            args.model,
            entry.id.as_str()
        )));
    };
    let Some(window) = model.model.context_window else {
        return Err(failed(format!(
            "{}'s window is unknown, so no limit applies to it; `demi providers models edit --context` gives it one",
            args.model
        )));
    };
    let limit = match args.size.parse::<ContextChoice>().map_err(failed)? {
        ContextChoice::Full => None,
        ContextChoice::Default => return Err(failed("default names the user's limit, which this sets: give a size, or full")),
        ContextChoice::Limit(tokens) if tokens > window => {
            return Err(failed(format!(
                "{} has a window of {}; give a size up to it, or full",
                args.model,
                context_word(window)
            )));
        }
        ContextChoice::Limit(tokens) => applied_context_limit(window, Some(tokens)),
    };
    let patch = PreferencesPatch {
        context_limit: Some(ContextLimitChange {
            provider_id: entry.id.clone(),
            model_id: args.model.clone(),
            tokens: limit,
        }),
        ..PreferencesPatch::default()
    };
    let checked = demi_backend_accounts::settings::check(patch).map_err(failed)?;
    let services = shard.services();
    shard.store().patch_preferences(move |saved| demi_backend_accounts::settings::merge(saved, checked))
        .await
        .map_err(failed)?;
    services.sync.mark(&user.id, Part::Preferences);
    let shown = ModelContext {
        entry: entry.label.clone(),
        model: args.model,
        window,
        limit,
    };
    if call.invocation.json {
        return json(&port, &shown).await;
    }
    let mut fields = Fields::new();
    fields
        .field("Entry", shown.entry.as_str())
        .field("Model", shown.model.as_str())
        .field("Window", context_word(window))
        .field("Context", limit.map_or_else(|| "full".to_owned(), context_word));
    out(&port, fields.render()).await
}

async fn hide(shard: Rc<Shard>, call: Call<HideArgs>, port: RpcPort) -> Result<u8, RpcError> {
    show_in_menu(shard, call, port, false).await
}

async fn unhide(shard: Rc<Shard>, call: Call<HideArgs>, port: RpcPort) -> Result<u8, RpcError> {
    show_in_menu(shard, call, port, true).await
}

/// Turns the entry, or its model `--model`, on or off in the model menu, as
/// Settings > Providers' Show in Model Menu does: the saved lists of what
/// the user turned off gain or lose it, read and written in one
/// transaction so a change made beside it stays.
async fn show_in_menu(
    shard: Rc<Shard>,
    call: Call<HideArgs>,
    port: RpcPort,
    shown_in_menu: bool,
) -> Result<u8, RpcError> {
    let args = call.args;
    let user = user(&shard).await?;
    let scope = shard.scope().await.map_err(failed)?;
    let entry = named(&scope.entries, &args.entry).map_err(failed)?;
    let id = entry.id.clone();
    if let Some(model) = &args.model {
        if shard.services().provider_registry.reads_directory(entry) {
            return Err(failed(format!(
                "--model: {} is shared by another server, whose settings say which of its models it offers",
                entry.label
            )));
        }
        let offered = scope
            .catalog_of(entry)
            .is_some_and(|catalog| catalog.models.iter().any(|offered| offered.model.id == *model));
        if !offered {
            return Err(failed(format!(
                "--model: {} offers no model {model}; `demi providers show {}` lists its models",
                entry.label,
                id.as_str()
            )));
        }
    }
    let services = shard.services();
    let provider = id.clone();
    shard.store().patch_preferences(move |mut saved| {
            match args.model {
                Some(model_id) => {
                    saved
                        .hidden_models
                        .retain(|hidden| !(hidden.provider_id == provider && hidden.model_id == model_id));
                    if !shown_in_menu {
                        saved.hidden_models.push(HiddenModel { provider_id: provider, model_id });
                    }
                }
                None => {
                    saved.hidden_providers.retain(|hidden| *hidden != provider);
                    if !shown_in_menu {
                        saved.hidden_providers.push(provider);
                    }
                }
            }
            saved
        })
        .await
        .map_err(failed)?;
    services.sync.mark(&user.id, Part::Preferences);
    let scope = shard.scope().await.map_err(failed)?;
    let entry = named(&scope.entries, id.as_str()).map_err(failed)?;
    let current = caller_model(&shard, &call.invocation);
    let shown = shown(&shard, &user, &scope, entry, current.as_ref()).await?;
    answer(&port, call.invocation.json, shown).await
}

/// A user who only infers with the entry sees no account, as in Settings.
async fn list_accounts(shard: Rc<Shard>, call: Call<EntryArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let user = user(&shard).await?;
    let entry = scoped(&shard, &user, &call.args.entry).await?;
    let disclose = Vault::manages(&user.id, &entry);
    accounts_answer(&shard, &port, call.invocation.json, &entry, disclose).await
}

async fn activate_account(shard: Rc<Shard>, call: Call<AccountArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let user = user(&shard).await?;
    let entry = managed(&shard, &user, &call.args.entry).await?;
    let services = shard.services();
    let listed = vault_accounts::list(&services.vault, &entry, true).await.map_err(failed)?;
    let account = account_named(&listed.accounts, &call.args.account)?;
    {
        let _held = services.hold_entry(&entry).map_err(failed)?;
        vault_accounts::activate(&services.vault, &services.provider_registry, &entry, account)
            .await
            .map_err(failed)?;
    }
    let entry = managed(&shard, &user, entry.id.as_str()).await?;
    accounts_answer(&shard, &port, call.invocation.json, &entry, true).await
}

async fn order_accounts(shard: Rc<Shard>, call: Call<AccountOrderArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let user = user(&shard).await?;
    let entry = managed(&shard, &user, &call.args.entry).await?;
    let services = shard.services();
    let listed = vault_accounts::list(&services.vault, &entry, true).await.map_err(failed)?;
    let order = call
        .args
        .account
        .iter()
        .map(|named| account_named(&listed.accounts, named))
        .collect::<Result<Vec<_>, _>>()?;
    {
        let _held = services.hold_entry(&entry).map_err(failed)?;
        vault_accounts::reorder(&services.vault, &entry, order)
            .await
            .map_err(failed)?;
    }
    accounts_answer(&shard, &port, call.invocation.json, &entry, true).await
}

/// The entry `named_as` names among the user's.
async fn scoped(shard: &Shard, user: &UserDto, named_as: &str) -> Result<ProviderEntry, RpcError> {
    let entries = entries(shard).await.map_err(failed)?;
    let id = named(&entries, named_as).map_err(failed)?.id.clone();
    shard
        .services()
        .scoped_entry(&user.id, id.as_str())
        .await
        .map_err(failed)
}

/// The entry `named_as` names, when the user manages it: changing its
/// accounts is the configuring user's, as in Settings.
async fn managed(shard: &Shard, user: &UserDto, named_as: &str) -> Result<ProviderEntry, RpcError> {
    let entries = entries(shard).await.map_err(failed)?;
    let id = named(&entries, named_as).map_err(failed)?.id.clone();
    shard
        .services()
        .managed_entry(&user.id, id.as_str())
        .await
        .map_err(failed)
}

/// The account `named` names among `accounts`: by its id, else by its
/// label, which must name one alone.
fn account_named(accounts: &[AccountInfo], named: &str) -> Result<CredentialId, RpcError> {
    let account = match accounts.iter().find(|account| account.id == named) {
        Some(account) => account,
        None => {
            let matching: Vec<&AccountInfo> = accounts.iter().filter(|account| account.label == named).collect();
            match matching.as_slice() {
                [account] => *account,
                [] => {
                    return Err(failed(format!(
                        "no account is labeled or has the id \"{named}\"; `demi providers accounts list` lists them"
                    )));
                }
                several => {
                    let ids: Vec<&str> = several.iter().map(|account| account.id.as_str()).collect();
                    return Err(failed(format!(
                        "several accounts are labeled \"{named}\"; name one by its id: {}",
                        ids.join(", ")
                    )));
                }
            }
        }
    };
    CredentialId::try_from(account.id.as_str()).map_err(failed)
}

/// The entry's accounts in their order, the active one marked; none unless
/// `disclose`.
async fn accounts_answer(
    shard: &Shard,
    port: &RpcPort,
    json_output: bool,
    entry: &ProviderEntry,
    disclose: bool,
) -> Result<u8, RpcError> {
    let listed = vault_accounts::list(&shard.services().vault, entry, disclose)
        .await
        .map_err(failed)?;
    let accounts: Vec<ListedAccount> = listed
        .accounts
        .into_iter()
        .map(|account| ListedAccount {
            active: listed.active.as_ref().is_some_and(|active| active.as_str() == account.id),
            id: account.id,
            label: account.label,
        })
        .collect();
    if json_output {
        return json(port, &accounts).await;
    }
    if !disclose {
        return out(port, format!("The accounts of {} are shown to the user who configures it.\n", entry.label)).await;
    }
    if accounts.is_empty() {
        return out(port, format!("{} has no account; the user signs in to one in Settings > Providers.\n", entry.label)).await;
    }
    let mut text = String::new();
    for account in &accounts {
        let mark = if account.active { " (active)" } else { "" };
        text.push_str(&format!("{}  {}{mark}\n", account.id, account.label));
    }
    out(port, text).await
}

async fn order(shard: Rc<Shard>, call: Call<OrderArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let user = user(&shard).await?;
    let entries = entries(&shard).await.map_err(failed)?;
    let mut ids = Vec::new();
    for name in &call.args.entry {
        let entry = named(&entries, name).map_err(failed)?;
        if !Vault::manages(&user.id, entry) {
            return Err(failed(ProviderRefusal::NotConfiguring));
        }
        if !ids.contains(&entry.id) {
            ids.push(entry.id.clone());
        }
    }
    // The order is of the user's own entries: another's, which a shared
    // instance lends them, keep the order their owner gave them.
    for entry in entries.iter().filter(|entry| Vault::manages(&user.id, entry)) {
        if !ids.contains(&entry.id) {
            ids.push(entry.id.clone());
        }
    }
    shard.services().order_providers(&user, ids).await.map_err(failed)?;
    list(shard, Call { args: ListArgs { all: Some(true) }, invocation: call.invocation }, port).await
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
        label: removed.label,
        removed: true,
    };
    if call.invocation.json {
        return json(&port, &removed).await;
    }
    out(&port, format!("Removed {} ({}).\n", removed.label, removed.id)).await
}

async fn search(shard: Rc<Shard>, call: Call<SearchArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let query = call.args.query.join(" ");
    let results = shard
        .services()
        .search_models(shard.user(), &query, None)
        .await
        .map_err(failed)?;
    if call.invocation.json {
        return json(&port, &results).await;
    }
    let mut table = Table::new([
        "model",
        "context (tokens)",
        "output (tokens)",
        "default effort",
        "efforts",
        "files",
        "name",
    ]);
    for found in &results {
        table.row::<Cell>([
            found.reference.as_str().into(),
            found.context_window.into(),
            found.output_limit.into(),
            found.thinking_efforts.first().into(),
            list_cell(&found.thinking_efforts).into(),
            files_cell(found.accepted_extensions.as_deref()),
            found.display_name.as_str().into(),
        ]);
    }
    out(&port, table.render()).await
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
    wire_fields_for(entry, &args.fields)?;
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
    let mut model = new_model(found.as_ref(), args.kind.unwrap_or_default(), args.fields)?;
    if started {
        // The vendor's own copy of the model gives way to the one added.
        models.retain(|listed| listed.id != model.id);
    }
    // A taken id takes the first free suffix (`crates/backend-user-shard/src/provider_settings.rs`).
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
    if started {
        let models = if count == 1 { "1 model".to_owned() } else { format!("{count} models") };
        let note = format!("{} now keeps its own list of {models}; it no longer follows models.dev", entry.label);
        port.stderr(warning(&call.invocation.path.join(" "), &note)).await?;
    }
    model_answer(&port, call.invocation.json, model).await
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
                kind: ModelKind::Text,
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
                thinking_form: model.thinking_form,
                interleaved_thinking: model.interleaved_thinking,
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
    wire_fields_for(entry, &args.fields)?;
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
    if let Some(form) = fields.thinking {
        model.thinking_form = form;
    }
    if let Some(switch) = fields.interleaved_thinking {
        model.interleaved_thinking = switch.on();
    }
    if let Some(tier) = fields.fast_tier {
        model.fast_tier = (tier != NONE).then_some(tier);
    }
    let model = model.clone();
    write_list(&shard, &user, entry, models).await?;
    model_answer(&port, call.invocation.json, model).await
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
    model_answer(&port, call.invocation.json, model).await
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
fn caller_model(shard: &Shard, invocation: &RpcInvocation) -> Option<ModelSelection> {
    let conversation = conversation_of(invocation).ok()?;
    let job = invocation.caller.as_ref()?;
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

/// Refuses `--thinking` and `--interleaved-thinking` but on a compatible
/// entry: a vendor's model takes both from models.dev, as Edit Model shows
/// neither for it.
fn wire_fields_for(entry: &ProviderEntry, fields: &ModelFields) -> Result<(), RpcError> {
    let compatible = matches!(&entry.credential, EntryCredential::ApiKey(config) if config.is_compatible());
    if compatible || fields.thinking.is_none() && fields.interleaved_thinking.is_none() {
        return Ok(());
    }
    Err(failed(format!(
        "--thinking and --interleaved-thinking set a compatible API's model; {}'s models take both from models.dev",
        entry.label
    )))
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
fn new_model(found: Option<&ModelMatch>, kind: ModelKind, fields: ModelFields) -> Result<ConfiguredModel, RpcError> {
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
    // A model that is not text has only its ID and name.
    if kind != ModelKind::Text {
        return Ok(ConfiguredModel {
            kind,
            id: Trimmed::from(id),
            display_name: Trimmed::from(display_name),
            context_window: None,
            output_limit: None,
            thinking_efforts: Vec::new(),
            accepted_extensions: None,
            fast_tier: None,
            thinking_form: ThinkingForm::default(),
            interleaved_thinking: false,
        });
    }
    Ok(ConfiguredModel {
        kind,
        id: Trimmed::from(id),
        display_name: Trimmed::from(display_name),
        context_window,
        output_limit,
        thinking_efforts,
        accepted_extensions,
        fast_tier: fields.fast_tier.filter(|tier| tier != NONE),
        thinking_form: fields
            .thinking
            .or_else(|| found.map(|found| found.thinking_form))
            .unwrap_or_default(),
        interleaved_thinking: fields
            .interleaved_thinking
            .map(Switch::on)
            .or_else(|| found.map(|found| found.interleaved_thinking))
            .unwrap_or(false),
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

/// The answer of a `models` change: the model as the list holds it now,
/// or as it held it before it was removed.
async fn model_answer(port: &RpcPort, json_output: bool, model: ConfiguredModel) -> Result<u8, RpcError> {
    if json_output {
        return json(port, &model).await;
    }
    let mut fields = Fields::new();
    fields
        .field("ID", model.id.as_str())
        .field("Name", model.display_name.as_str())
        .field("Context (tokens)", model.context_window)
        .field("Output (tokens)", model.output_limit)
        .field("Default effort", model.thinking_efforts.first())
        .field("Efforts", list_cell(&model.thinking_efforts))
        .field("Files", files_cell(model.accepted_extensions.as_deref()))
        .field("Fast tier", model.fast_tier.as_deref());
    out(port, fields.render()).await
}

/// The entry as `show` and a change answer it.
async fn shown(
    shard: &Shard,
    user: &UserDto,
    scope: &Scope,
    entry: &ProviderEntry,
    current: Option<&ModelSelection>,
) -> Result<ShownEntry, RpcError> {
    let services = shard.services();
    let accounts = match entry.credential {
        EntryCredential::Subscription { .. } if Vault::manages(&user.id, entry) => services
            .vault
            .accounts(entry.id.clone())
            .await
            .map_err(failed)?
            .into_iter()
            .map(|account| account.label)
            .collect(),
        _ => Vec::new(),
    };
    let models_from = match &entry.credential {
        // A compatible entry lists only its own models, none at first.
        EntryCredential::ApiKey(config) if config.models.is_some() || config.is_compatible() => ModelSource::List,
        EntryCredential::ApiKey(_) => ModelSource::Vendor,
        EntryCredential::Subscription { .. } => ModelSource::Provider,
    };
    let catalog = scope.catalog_of(entry);
    let models = catalog
        .map(|catalog| {
            catalog
                .models
                .iter()
                .map(|model| ShownModel {
                    id: model.model.id.clone(),
                    display_name: model.model.display_name.clone(),
                    context_window: model.model.context_window,
                    output_limit: model.model.output_limit,
                    efforts: model.model.efforts(),
                    can_turn_off: model.model.can_turn_thinking_off(),
                    tiers: tiers(&model.model.service_tiers),
                    accepted_extensions: model.model.accepted_extensions.clone(),
                    hidden: scope.preferences.hides_model(&entry.id, &model.model.id),
                    current: current.is_some_and(|current| {
                        current.provider_id == entry.id.as_str() && current.model.id == model.model.id
                    }),
                })
                .collect()
        })
        .unwrap_or_default();
    let through_name = match &entry.through_device {
        Some(id) => Some(
            shard
                .host_records()
                .await
                .map_err(failed)?
                .into_iter()
                .find(|device| device.id == *id)
                .map_or_else(|| id.as_str().to_owned(), |device| device.name),
        ),
        None => None,
    };
    Ok(ShownEntry {
        entry: services.provider_registry.provider_dto(entry),
        status: catalog.map_or_else(
            || Availability::setup(services.provider_registry.missing(entry)),
            |catalog| catalog.availability.clone(),
        ),
        hidden: scope.preferences.hides_provider(&entry.id),
        accounts,
        models_from,
        catalog: models,
        through_name,
    })
}

async fn answer(port: &RpcPort, json_output: bool, shown: ShownEntry) -> Result<u8, RpcError> {
    if json_output {
        return json(port, &shown).await;
    }
    out(port, shown.render()).await
}

fn tiers(tiers: &[demi_shared_types::ServiceTier]) -> Vec<String> {
    tiers.iter().map(|tier| tier.id.clone()).collect()
}

/// A list of values as one cell, comma-separated; empty when it has none.
fn list_cell(values: &[String]) -> String {
    values.join(",")
}

/// Efforts as a cell: the model's, and `off` last when thinking can be
/// turned off.
fn efforts_cell(efforts: &[String], can_turn_off: bool) -> String {
    let mut words = efforts.to_vec();
    if can_turn_off {
        words.push(EFFORT_OFF.to_owned());
    }
    list_cell(&words)
}

/// The files a model reads as a cell: missing when unknown, `none` for no
/// file.
fn files_cell(extensions: Option<&[FileExtension]>) -> Cell {
    match extensions {
        None => Cell::from(None::<String>),
        Some([]) => "none".into(),
        Some(extensions) => extensions
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(",")
            .into(),
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

impl ShownEntry {
    /// The text `show` prints: a setting per line, then a row per model.
    fn render(&self) -> String {
        let mut accounts = Table::new(["account"]);
        for account in &self.accounts {
            accounts.row([account.as_str()]);
        }
        let entry = &self.entry;
        // A vendor entry lists each header its vendor documents, empty when
        // it sends none; a compatible entry sends an empty value too.
        let mut headers = Table::new(["header", "value"]);
        let mut sent = 0;
        for header in entry.headers.iter().filter(|header| entry.compatible || !header.value.is_empty()) {
            headers.row([header.name.as_str(), header.value.as_str()]);
            sent += 1;
        }
        let mut models = Table::new([
            "id",
            "context (tokens)",
            "output (tokens)",
            "default effort",
            "efforts",
            "tiers",
            "files",
            "current",
            "hidden",
            "name",
        ]);
        for model in &self.catalog {
            models.row::<Cell>([
                model.id.as_str().into(),
                model.context_window.into(),
                model.output_limit.into(),
                model.efforts.first().into(),
                efforts_cell(&model.efforts, model.can_turn_off).into(),
                list_cell(&model.tiers).into(),
                files_cell(model.accepted_extensions.as_deref()),
                model.current.into(),
                model.hidden.into(),
                model.display_name.as_str().into(),
            ]);
        }
        let api_key = entry.kind == CredentialKind::ApiKey;
        let switch = |on: bool| if on { "on" } else { "off" };
        let search = [
            ("the web", entry.search.web),
            ("X", entry.search.x),
            ("images", entry.search.images),
            ("videos", entry.search.videos),
        ]
        .into_iter()
        .filter_map(|(what, on)| Some(format!("{what} {}", switch(on?))))
        .collect::<Vec<_>>();
        let mut fields = Fields::new();
        fields
            .field("ID", entry.id.as_str())
            .field("Label", entry.label.as_str())
            .field("Kind", kind_text(entry.kind))
            .field("Vendor", entry.vendor_id.as_deref())
            .field("Family", entry.provider_type.as_str())
            .field("Wire", entry.wire_api.map(|wire| wire.to_string()))
            .field("Endpoint", entry.base_url.as_ref().map(|url| url.url().to_string()))
            .field(
                "Connect through",
                entry.connects_through.then(|| self.through_name.as_deref().unwrap_or("Server")),
            )
            .field("Key saved", api_key.then_some(entry.has_api_key));
        if sent > 0 {
            fields.table("Headers", &headers);
        }
        if entry.kind == CredentialKind::Subscription {
            fields.table("Accounts", &accounts);
            fields.field("Rotation", entry.rotation.to_string());
        }
        fields
            .field(
                "Prompt cache lifetime",
                entry.cache_ttl_presets.is_some().then(|| entry.cache_ttl.as_ref().map_or("unspecified", |ttl| ttl.as_str())),
            )
            .field("Searches", (!search.is_empty()).then(|| search.join(", ")))
            .field("Status", state(&self.status))
            .field("Hidden", self.hidden)
            .field(
                "Images",
                entry.media.image.shape.map(|shape| format!("{shape}, Image Generation {}", switch(entry.media.image.enabled))),
            )
            .field(
                "Videos",
                entry.media.video.shape.map(|shape| format!("{shape}, Video Generation {}", switch(entry.media.video.enabled))),
            )
            .field(
                "Audio",
                entry.media.audio.shape.map(|shape| format!("{shape}, Audio Generation {}", switch(entry.media.audio.enabled))),
            )
            .field(
                "Models from",
                match self.models_from {
                    ModelSource::List => "list",
                    ModelSource::Vendor => "vendor",
                    ModelSource::Provider => "provider",
                },
            )
            .table("Models", &models);
        let audio: Vec<&ConfiguredModel> =
            entry.models.iter().flat_map(|models| &models.0).filter(|model| model.kind == ModelKind::Audio).collect();
        if !audio.is_empty() {
            let mut table = Table::new(["id", "name"]);
            for model in audio {
                table.row([model.id.as_str(), model.display_name.as_str()]);
            }
            fields.table("Audio models", &table);
        }
        fields.render()
    }
}
