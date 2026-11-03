//! The product's `demi integrations` group: the master's integrations, as
//! Settings › Integrations shows and changes them, in the master's own
//! conversations only.
//!
//! `edit` takes the integration `show --json` prints as a JSON merge patch,
//! so every setting has one name here, the protocol's, and a new setting
//! needs no new option. A change goes through the shard's methods the web
//! routes call (`integrations.rs`), with their checks.
//!
//! No command prints a token. `add` and `token` save the new one in the
//! user's vault and print the tag of its card (`<credential name="…" />`),
//! which a reply shows the user: in the credential that holds the
//! integration's token, found by the token's hash whatever it was renamed,
//! or a new one named *<name> token*, *<name> token 2* when that name is
//! taken, so no credential of the user's is overwritten. The credential is
//! chosen before the token is made, so a token is never made that no
//! credential can take. `remove` removes that credential.

use std::collections::BTreeMap;
use std::rc::{Rc, Weak};

use demi_backend_database::accounts::TokenHash;
use demi_backend_database::permissions::CredentialAsk;
use demi_backend_permissions::save_credential;
use demi_host_interface::{Call, GroupBuilder, LeafBuilder, RpcError, RpcInvocation, RpcPort, TypedRpc};
use demi_shared_types::output::{Cell, Fields, Table};
use demi_web_api_protocol::credentials::{CredentialField, FieldKind, Keep};
use demi_web_api_protocol::error::problems;
use demi_web_api_protocol::integrations::{IntegrationDto, IntegrationPatch, IntegrationWithToken};
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::Shard;
use super::credentials::CredentialError;
use crate::conversation::organize::{failed, verb};
use demi_backend_database::credentials::CredentialRefusal;

/// The permission category of the commands that change the integrations.
pub(crate) const MANAGE: &str = "integrations.manage";

const MANAGE_ACTION: &str = "manage integrations";

const MANAGE_DESCRIPTION: &str = "Add, change and remove your integrations, their tokens and the sizes of their Clouds.";

const SUMMARY: &str = "The user's integrations: the programs that build an agent product on Demi through its API, as Settings › Integrations has them. List and show them; add, edit and remove one; give one a new token. A change needs the user's permission.";

/// The group's entry in the model's capability index.
const ENTRY: &str = "The user's integrations, the programs that use Demi through its API.";

const LIST_SUMMARY: &str = "List each integration: its id, its name, and how many Clouds and conversations it holds. A read.";

const SHOW_SUMMARY: &str = "Show every setting of one integration, as Settings › Integrations has it (never its token): with --json the object `edit` changes. A read.";

const ADD_SUMMARY: &str = "Add an integration with every setting at its default, named Integration, Integration 2 when that name is taken; `edit` sets the rest. Its token is saved as the credential \u{201c}<name> token\u{201d}, whose card tag it prints. Needs the user's permission.";

const EDIT_SUMMARY: &str = "Change any setting of an integration, by a JSON merge patch on stdin of the object `demi integrations show --json` prints: a field given takes its value, a field left out stays, and null sets initScript, cloudIdleMinutes or a field of cloudSizing back to the server's; no other field takes null. id, tokenFingerprint, createdAt, clouds and conversations are the integration's own. Needs the user's permission.";

const TOKEN_SUMMARY: &str = "Give an integration a new token; the old one stops at once, and its program's connections close. The token is saved as the credential \u{201c}<name> token\u{201d}, whose card tag it prints. Needs the user's permission.";

const REMOVE_SUMMARY: &str = "Delete an integration with its conversations, its Clouds and its shared folder, as Settings does, and the credential of its token while it holds this integration's. Only when the user asks for it. Needs the user's permission.";

const INTEGRATION_ARG: &str = "The integration's name or id, as `demi integrations list` shows them.";
const PATCH: &str = "The changes, a JSON object such as {\"capabilities\": {\"browser\": false}, \"cloudSizing\": {\"memoryMib\": 4096}}.";

/// The fields `show --json` prints that no edit changes.
const OWN: [&str; 5] = ["id", "tokenFingerprint", "createdAt", "clouds", "conversations"];

/// The settings that null sets back to the server's.
const NULLABLE: [&str; 2] = ["initScript", "cloudIdleMinutes"];

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

/// The input of `show`, `token` and `remove`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct IntegrationArgs {
    integration: String,
}

/// The input of `edit`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EditArgs {
    integration: String,
    /// The patch, from standard input.
    patch: Option<String>,
}

/// What `add` and `token` print with --json: the integration, and the
/// credential its token is saved as.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Saved {
    integration: IntegrationDto,
    credential: String,
}

/// What `remove` prints with --json.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Removed {
    integration: IntegrationDto,
    /// The credential of its token that went with it; null when none did.
    credential: Option<String>,
}

pub(crate) fn integrations_group(shard: Weak<Shard>) -> GroupBuilder {
    GroupBuilder::new("integrations", SUMMARY)
        .index_entry(ENTRY)
        .permission(MANAGE, MANAGE_ACTION, MANAGE_DESCRIPTION)
        .leaf(
            LeafBuilder::rpc("list", LIST_SUMMARY)
                .input::<NoArgs>()
                .json_output::<Vec<IntegrationDto>>()
                .bind(TypedRpc::new(verb(shard.clone(), list))),
        )
        .leaf(
            LeafBuilder::rpc("show", SHOW_SUMMARY)
                .input::<IntegrationArgs>()
                .describe("integration", INTEGRATION_ARG)
                .positionals(["integration"])
                .json_output::<IntegrationDto>()
                .bind(TypedRpc::new(verb(shard.clone(), show))),
        )
        .leaf(
            LeafBuilder::rpc("add", ADD_SUMMARY)
                .input::<NoArgs>()
                .json_output::<Saved>()
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard.clone(), add))),
        )
        .leaf(
            LeafBuilder::rpc("edit", EDIT_SUMMARY)
                .input::<EditArgs>()
                .describe("integration", INTEGRATION_ARG)
                .describe("patch", PATCH)
                .positionals(["integration"])
                .stdin_field("patch")
                .json_output::<IntegrationDto>()
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard.clone(), edit))),
        )
        .leaf(
            LeafBuilder::rpc("token", TOKEN_SUMMARY)
                .input::<IntegrationArgs>()
                .describe("integration", INTEGRATION_ARG)
                .positionals(["integration"])
                .json_output::<Saved>()
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard.clone(), token))),
        )
        .leaf(
            LeafBuilder::rpc("remove", REMOVE_SUMMARY)
                .input::<IntegrationArgs>()
                .describe("integration", INTEGRATION_ARG)
                .positionals(["integration"])
                .json_output::<Removed>()
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard, remove))),
        )
}

async fn list(shard: Rc<Shard>, call: Call<NoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let integrations = shard.integrations().await.map_err(failed)?;
    if call.invocation.json {
        return json(&port, &integrations).await;
    }
    let mut table = Table::new(["id", "name", "clouds", "conversations"]);
    for integration in &integrations {
        table.row::<Cell>([
            integration.id.as_str().into(),
            (&integration.name).into(),
            integration.clouds.into(),
            integration.conversations.into(),
        ]);
    }
    if table.is_empty() {
        return out(&port, "The user has no integrations.\n".to_owned()).await;
    }
    out(&port, table.render()).await
}

async fn show(shard: Rc<Shard>, call: Call<IntegrationArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let integration = named(&shard, &call.args.integration).await?;
    answer(&port, call.invocation.json, integration).await
}

async fn add(shard: Rc<Shard>, call: Call<NoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let created = shard.create_integration().await.map_err(failed)?;
    let name = free_name(&shard, &created.integration.name).await?;
    saved(&shard, &call.invocation, &port, created, name, "Added").await
}

async fn edit(shard: Rc<Shard>, call: Call<EditArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let args = call.args;
    let integration = named(&shard, &args.integration).await?;
    let id = integration.id.clone();
    let patch = integration_patch(&integration, args.patch.as_deref().unwrap_or_default())?;
    let integration = match patch {
        Some(patch) => {
            patch.validate().map_err(|report| failed(problems(&report)))?;
            shard
                .update_integration(id, patch)
                .await
                .map_err(failed)?
                .ok_or_else(|| failed("the integration is gone"))?
        }
        None => {
            let command = call.invocation.path.join(" ");
            let what = "no change: every setting holds the value given";
            port.stderr(demi_shared_types::output::note(&command, what)).await?;
            integration
        }
    };
    answer(&port, call.invocation.json, integration).await
}

async fn token(shard: Rc<Shard>, call: Call<IntegrationArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let integration = named(&shard, &call.args.integration).await?;
    // Found by the old token, which the replacement ends.
    let name = match token_credential(&shard, &integration).await? {
        Some((_, holder)) => holder,
        None => free_name(&shard, &integration.name).await?,
    };
    let replaced = shard
        .replace_integration_token(integration.id)
        .await
        .map_err(failed)?
        .ok_or_else(|| failed("the integration is gone"))?;
    saved(&shard, &call.invocation, &port, replaced, name, "Gave a new token to").await
}

async fn remove(shard: Rc<Shard>, call: Call<IntegrationArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let integration = named(&shard, &call.args.integration).await?;
    // Read before the deletion, which ends the token.
    let credential = token_credential(&shard, &integration).await?;
    if !shard.delete_integration(&integration.id).await.map_err(failed)? {
        return Err(failed("the integration is gone"));
    }
    let credential = match credential {
        Some((id, name)) => match shard.remove_credential(id).await {
            Ok(()) => Some(name),
            // Removed beside this command: nothing is left to remove.
            Err(CredentialError::Refused(CredentialRefusal::NotFound)) => None,
            Err(error) => {
                return Err(failed(format!(
                    "deleted the integration \u{201c}{}\u{201d}, but not the credential \u{201c}{name}\u{201d} of its token: {error}",
                    integration.name
                )));
            }
        },
        None => None,
    };
    if call.invocation.json {
        return json(&port, &Removed { integration, credential }).await;
    }
    let mut text = format!(
        "Deleted the integration \u{201c}{}\u{201d} with its {} and {}.\n",
        integration.name,
        counted(integration.conversations, "conversation"),
        counted(integration.clouds, "Cloud"),
    );
    if let Some(name) = credential {
        text.push_str(&format!("Removed the credential \u{201c}{name}\u{201d}, which held its token.\n"));
    }
    out(&port, text).await
}

/// The user's credential of one field, `token`, that holds the token of
/// `integration`, with its id, whatever it is named; none when no such
/// credential does, as when the user changed it. A credential of other
/// fields is the user's own, which no command here changes.
async fn token_credential(
    shard: &Shard,
    integration: &IntegrationDto,
) -> Result<Option<(demi_web_api_protocol::ids::UserCredentialId, String)>, RpcError> {
    let control = &shard.services().control;
    let credentials = control.user_credentials(shard.user().clone()).await.map_err(failed)?;
    for credential in credentials {
        if !matches!(credential.fields.as_slice(), [field] if field.name == "token") {
            continue;
        }
        let Some((stored, values)) = control
            .user_credential_values(shard.user().clone(), credential.id, shard.vault_seal())
            .await
            .map_err(failed)?
        else {
            continue;
        };
        let Some(value) = values.get("token") else {
            continue;
        };
        let holder = control.integration_by_token(TokenHash::of(value)).await.map_err(failed)?;
        if holder.is_some_and(|holder| holder.id == integration.id) {
            return Ok(Some((stored.id, stored.name)));
        }
    }
    Ok(None)
}

/// The name of a new credential for the token of the integration
/// `integration`: *<name> token*, or *<name> token 2* and on while the
/// user has a credential of that name, whatever its case.
async fn free_name(shard: &Shard, integration: &str) -> Result<String, RpcError> {
    let control = &shard.services().control;
    let base = format!("{integration} token");
    let mut name = base.clone();
    for number in 2.. {
        if control
            .user_credential_named(shard.user().clone(), name.clone())
            .await
            .map_err(failed)?
            .is_none()
        {
            break;
        }
        name = format!("{base} {number}");
    }
    Ok(name)
}

/// Saves the token of `with` in the credential `name`, the one that held
/// the old token or a name no credential has, and prints what `did` says
/// was done, the credential's name and the tag of its card.
async fn saved(
    shard: &Shard,
    invocation: &RpcInvocation,
    port: &RpcPort,
    with: IntegrationWithToken,
    name: String,
    did: &str,
) -> Result<u8, RpcError> {
    let IntegrationWithToken { integration, token } = with;
    let ask = CredentialAsk {
        reason: format!("The token of the integration \u{201c}{}\u{201d}", integration.name),
        fields: vec![CredentialField {
            name: "token".to_owned(),
            label: "Token".to_owned().into(),
            kind: FieldKind::Secret,
        }],
        keep: Keep::Always,
    };
    let values = BTreeMap::from([("token".to_owned(), token)]);
    let answered = save_credential(shard.permission_shard(), invocation, name, ask, values).await?;
    let credential = answered.credential.name;
    if invocation.json {
        return json(port, &Saved { integration, credential }).await;
    }
    out(
        port,
        format!(
            "{did} the integration \u{201c}{}\u{201d} ({}). Its token is saved as the credential \u{201c}{credential}\u{201d}; a reply shows it to the user with:\n{}\n",
            integration.name,
            integration.id.as_str(),
            card_tag(&credential),
        ),
    )
    .await
}

/// The tag of the card of the credential `name`, escaped as XML escapes an
/// attribute.
fn card_tag(name: &str) -> String {
    let name = name
        .replace('&', "&amp;")
        .replace('"', "&quot;")
        .replace('<', "&lt;")
        .replace('>', "&gt;");
    format!("<credential name=\"{name}\" />")
}

/// The user's integration `name` names: the one of that id, else the one
/// of that name, whatever its case.
async fn named(shard: &Shard, name: &str) -> Result<IntegrationDto, RpcError> {
    let mut integrations = shard.integrations().await.map_err(failed)?;
    if let Some(index) = integrations.iter().position(|integration| integration.id.as_str() == name) {
        return Ok(integrations.swap_remove(index));
    }
    let wanted = name.to_lowercase();
    let mut matches: Vec<IntegrationDto> = integrations
        .into_iter()
        .filter(|integration| integration.name.to_lowercase() == wanted)
        .collect();
    match matches.len() {
        0 => Err(failed(format!(
            "\u{201c}{name}\u{201d}: no integration of that name or id; `demi integrations list` lists them"
        ))),
        1 => Ok(matches.remove(0)),
        _ => Err(failed(format!(
            "\u{201c}{name}\u{201d}: several integrations have that name; name one by its id, which `demi integrations list` shows"
        ))),
    }
}

/// The patch that `text`, a JSON merge patch of the integration `shown`
/// prints (RFC 7396), asks for; none when it changes nothing.
///
/// - An object is merged into the one shown and then sent whole, as the
///   protocol replaces `capabilities` and `cloudSizing`; null in it sets
///   that field back to the server's.
/// - A field that holds the value shown changes nothing.
/// - A field of the integration's own ([`OWN`]) is refused when it holds
///   another value, and one `show` does not print is refused by its name.
fn integration_patch(shown: &IntegrationDto, text: &str) -> Result<Option<IntegrationPatch>, RpcError> {
    const EXAMPLE: &str = "stdin: give the changes as a JSON object, such as {\"name\": \"Support desk\"}";
    let patch: Value = serde_json::from_str(text).map_err(|error| failed(format!("{EXAMPLE}: {error}")))?;
    let Value::Object(patch) = patch else {
        return Err(failed(EXAMPLE));
    };
    let current = serde_json::to_value(shown).expect("an integration serializes");
    let mut changes = Map::new();
    for (field, value) in patch {
        let Some(was) = current.get(&field) else {
            return Err(failed(format!(
                "{field}: no such setting; `demi integrations show --json` prints each"
            )));
        };
        let merged = merged(was, value);
        if merged == *was {
            continue;
        }
        if OWN.contains(&field.as_str()) {
            return Err(failed(format!("{field}: the integration's own, which no edit changes")));
        }
        if merged.is_null() && !NULLABLE.contains(&field.as_str()) {
            return Err(failed(format!(
                "{field}: takes a value; null sets only initScript, cloudIdleMinutes and the fields of cloudSizing back to the server's"
            )));
        }
        changes.insert(field, merged);
    }
    if changes.is_empty() {
        return Ok(None);
    }
    serde_json::from_value(Value::Object(changes))
        .map(Some)
        .map_err(|error| failed(format!("stdin: {error}")))
}

/// `patch` merged into `was`: an object's fields one by one, a null among
/// them a null, anything else in its place.
fn merged(was: &Value, patch: Value) -> Value {
    match (was, patch) {
        (Value::Object(was), Value::Object(patch)) => {
            let mut fields = was.clone();
            for (field, value) in patch {
                let next = match fields.get(&field) {
                    Some(old) => merged(old, value),
                    None => value,
                };
                fields.insert(field, next);
            }
            Value::Object(fields)
        }
        (_, patch) => patch,
    }
}

async fn answer(port: &RpcPort, json_output: bool, integration: IntegrationDto) -> Result<u8, RpcError> {
    if json_output {
        return json(port, &integration).await;
    }
    out(port, rendered(&integration)).await
}

/// An integration as `show` prints it.
fn rendered(integration: &IntegrationDto) -> String {
    let server = "the server's";
    let sizing = &integration.cloud_sizing;
    let capabilities = serde_json::to_value(integration.capabilities).expect("capabilities serialize");
    let on: Vec<&str> = capabilities
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, value)| value.as_bool() == Some(true))
        .map(|(name, _)| name.as_str())
        .collect();
    let mut fields = Fields::new();
    fields
        .field("id", integration.id.as_str())
        .field("name", &integration.name)
        .field("created", integration.created_at)
        .field("capabilities", if on.is_empty() { "none".to_owned() } else { on.join(", ") })
        .field("shared folder", &integration.shared_path)
        .field(
            "idle minutes",
            integration.cloud_idle_minutes.map_or(server.to_owned(), |minutes| minutes.to_string()),
        )
        .field("cpus", sizing.cpus.map_or(server.to_owned(), |cpus| cpus.to_string()))
        .field("memory MiB", sizing.memory_mib.map_or(server.to_owned(), |memory| memory.to_string()))
        .field("system disk GB", sizing.system_gb.map_or(server.to_owned(), |gb| gb.to_string()))
        .field("home disk GB", sizing.home_gb.map_or(server.to_owned(), |gb| gb.to_string()))
        .field("clouds", integration.clouds)
        .field("conversations", integration.conversations);
    match &integration.init_script {
        Some(script) => fields.text("init script", script),
        None => fields.field("init script", "none"),
    };
    fields.render()
}

/// `count` of `noun`, its plural for any but one.
fn counted(count: u32, noun: &str) -> String {
    if count == 1 { format!("1 {noun}") } else { format!("{count} {noun}s") }
}

async fn out(port: &RpcPort, text: String) -> Result<u8, RpcError> {
    port.stdout(text.into_bytes()).await?;
    Ok(0)
}

async fn json(port: &RpcPort, value: &impl Serialize) -> Result<u8, RpcError> {
    let text = serde_json::to_string(value).expect("a command's output serializes");
    out(port, format!("{text}\n")).await
}
