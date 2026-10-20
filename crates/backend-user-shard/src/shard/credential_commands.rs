//! The product's `demi credentials` group: an agent lists the user's
//! credentials and shows one, asks the user for one and waits for the
//! answer, reads one field's value onto its standard output once the
//! conversation may use the credential, and, with the user's Manage
//! Credentials, removes one. No output of these commands carries a secret
//! value but `read`'s, which the agent passes on.
//!
//! An agent that needs a secret asks the user for it with `request` rather
//! than reading it from the message, where it would sit in the transcript and
//! reach the model's vendor. `read` writes the value to its standard output,
//! wherever that goes: Demi refuses and masks nothing, and how the value is
//! passed on is the agent's choice. A field's kind is advice, not a rule: the
//! command's help recommends passing a secret where the model does not read
//! it, into a pipe, a file or a `$(…)`, since a value the job prints joins the
//! transcript. The command line the model wrote, which the transcript keeps,
//! holds the `$(…)` as written, never its value, and Demi writes no value
//! into the transcript, a log or a frame to the page, but the Settings page's
//! reveal. A value enters the transcript only when an agent's command prints
//! it.
//!
//! A credential is used in a conversation only once the user allowed it there:
//! one the user saved from that conversation's request is allowed at once, any
//! other asks first, and an allowance lasts the conversation's life for every
//! agent of its tree. How long a credential is kept is the agent's decision,
//! with the request's `--keep`, since it knows what the secret is for; the
//! card offers no choice, as a form does not ask how long its data is kept.

use std::collections::BTreeSet;
use std::rc::{Rc, Weak};

use demi_backend_database::credentials::StoredCredential;
use demi_backend_database::permissions::CredentialAsk;
use demi_backend_permissions::{Answer, ask_credential, read_credential};
use demi_backend_host_access::host_commands::conversation_of;
use demi_host_interface::{
    Call, GroupBuilder, LeafBuilder, RpcError, RpcInvocation, RpcPort, TypedRpc,
};
use demi_shared_types::{
    Nullable, Timestamp,
    output::{Cell, Fields, Table},
};
use demi_web_api_protocol::credentials::{
    CredentialField, FIELDS_MAX, FieldKind, Keep, NAME_MAX, distinct,
};
use demi_web_api_protocol::ids::UserCredentialId;
use demi_web_api_protocol::text::Trimmed;
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Shard;
use super::credentials::CredentialError;
use crate::conversation::organize::{failed, verb};

/// The permission category of `remove`.
pub(crate) const MANAGE: &str = "credentials.manage";

const MANAGE_ACTION: &str = "manage credentials";

const MANAGE_DESCRIPTION: &str = "Manage Credentials lets the agents of this conversation remove your saved credentials. Your credentials serve every conversation you allow them in.";

const SUMMARY: &str = "The user's credentials: named secrets such as API keys, tokens and passwords, each with fields. Ask the user for one, pass a field's value on without reading it, list and show them, or remove one.";

/// The group's entry in the model's capability index.
const ENTRY: &str = "Keeps the secrets a task needs, API keys, tokens, passwords, as the user's named credentials: `request` asks the user for one on a card and waits for the answer; `read` writes one field's value to standard output, to pass into a pipe, a file or `$(…)` without printing it. Use it whenever a task needs a secret, rather than asking the user to paste it into the conversation, where it would join the transcript.";

const LIST_SUMMARY: &str = "List the user's credentials: name, field names, when each is removed (- for kept always), and whether this conversation may use it. No value.";

const SHOW_SUMMARY: &str = "Show one credential: its fields, the values of its visible fields once this conversation may use it, its note and its dates. A secret field's value is never shown; `read` passes it on.";

const REQUEST_SUMMARY: &str = "Ask the user for a credential on a card in the conversation, or for new values of the saved one of that name, which then must have the same fields. Waits for the user's answer, however long, and prints what was saved, or fails when the user declines. The credential is then allowed in this conversation.";

const READ_SUMMARY: &str = "Write one field's value to standard output, exactly as saved, without a newline. A value you print is sent to the model's vendor and stays in the transcript; to avoid that, pipe it into a command (`demi credentials read \"GitHub token\" token | gh auth login --with-token`), redirect it into a file (`… > token.txt`), or substitute it (`TOKEN=$(demi credentials read \"GitHub token\" token) git push`). A credential this conversation may not use yet asks the user first, and the read fails until they allow it.";

const REMOVE_SUMMARY: &str = "Remove a credential, its values and every conversation's allowance to use it. Only when the user asks for it. Needs the user's permission.";

const RUNNING_HINT: &str = "Waiting for the user to answer the credential card in the conversation, which may take a while. End your turn and say what you are waiting for; this command reports when the user answers.";

/// The input of `list`, which takes none.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

/// The input of `show` and `remove`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NameArgs {
    name: String,
}

/// The input of `request`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RequestArgs {
    name: String,
    #[schemars(length(min = 1, max = FIELDS_MAX))]
    field: Vec<String>,
    reason: Option<String>,
    keep: Option<Keep>,
}

/// The input of `read`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    name: String,
    field: String,
}

/// A credential as `list` and `request` print it: no value.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Listed {
    name: String,
    fields: Vec<CredentialField>,
    /// When it is removed; null while it is kept always.
    #[schemars(with = "Nullable<Timestamp>")]
    keep_until: Option<Timestamp>,
    /// Whether this conversation may use it.
    allowed: bool,
}

/// A field as `show` prints it: a visible field's value once the
/// conversation may use the credential, never a secret one's.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct ShownField {
    name: String,
    label: String,
    kind: FieldKind,
    #[schemars(with = "Nullable<String>")]
    value: Option<String>,
}

/// A credential as `show` prints it.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Shown {
    name: String,
    note: String,
    fields: Vec<ShownField>,
    #[schemars(with = "Nullable<Timestamp>")]
    keep_until: Option<Timestamp>,
    #[schemars(with = "Nullable<Timestamp>")]
    last_used_at: Option<Timestamp>,
    created_at: Timestamp,
    updated_at: Timestamp,
    allowed: bool,
}

/// The `credentials` group, whose handlers act in `shard`.
pub(crate) fn credentials_group(shard: Weak<Shard>) -> GroupBuilder {
    GroupBuilder::new("credentials", SUMMARY)
        .index_entry(ENTRY)
        .permission(MANAGE, MANAGE_ACTION, MANAGE_DESCRIPTION)
        .leaf(
            LeafBuilder::rpc("list", LIST_SUMMARY)
                .input::<NoArgs>()
                .json_output::<Vec<Listed>>()
                .bind(TypedRpc::new(verb(shard.clone(), list))),
        )
        .leaf(
            LeafBuilder::rpc("show", SHOW_SUMMARY)
                .input::<NameArgs>()
                .describe("name", "The credential's name.")
                .positionals(["name"])
                .json_output::<Shown>()
                .bind(TypedRpc::new(verb(shard.clone(), show))),
        )
        .leaf(
            LeafBuilder::rpc("request", REQUEST_SUMMARY)
                .input::<RequestArgs>()
                .describe(
                    "name",
                    format!("The credential's name, 1 to {NAME_MAX} characters, such as \"GitHub token for deploys\"."),
                )
                .describe(
                    "field",
                    "One field as <name>:<secret|visible>:<label>, such as token:secret:\"Personal access token\" or user:visible:\"User name\"; one to ten. A name is lowercase letters, digits, _ and -. A secret field is typed hidden and never listed; a visible one, such as a user name or a host, is shown as it is.",
                )
                .describe("reason", "Why you ask, which the card shows the user and the credential keeps as its note.")
                .describe("keep", "How long the credential is kept, which you decide by what it is for and the card tells the user: 1h (the default) for a secret used once, such as a key to add a provider; 1d or 30d for work that goes on that long; always for one the user wants you to keep using, which stays in Settings › Credentials.")
                .positionals(["name"])
                .running_hint(RUNNING_HINT)
                .json_output::<Listed>()
                .bind(TypedRpc::new(verb(shard.clone(), request))),
        )
        .leaf(
            LeafBuilder::rpc("read", READ_SUMMARY)
                .input::<ReadArgs>()
                .describe("name", "The credential's name.")
                .describe("field", "The field's name.")
                .positionals(["name", "field"])
                .bind(TypedRpc::new(verb(shard.clone(), read))),
        )
        .leaf(
            LeafBuilder::rpc("remove", REMOVE_SUMMARY)
                .input::<NameArgs>()
                .describe("name", "The credential's name.")
                .positionals(["name"])
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard, remove))),
        )
}

/// The ids of the credentials the call's conversation may use.
async fn allowed(shard: &Shard, invocation: &RpcInvocation) -> Result<BTreeSet<UserCredentialId>, RpcError> {
    shard
        .services()
        .control
        .allowed_credentials(conversation_of(invocation)?)
        .await
        .map_err(failed)
}

fn listed(credential: &StoredCredential, allowed: bool) -> Listed {
    Listed {
        name: credential.name.clone(),
        fields: credential.fields.clone(),
        keep_until: credential.keep_until,
        allowed,
    }
}

/// A credential's field names as one cell: `token,user`.
fn field_names(fields: &[CredentialField]) -> String {
    fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<Vec<_>>()
        .join(",")
}

/// A credential's fields as a table: name, kind, label, and the value a
/// visible field shows, missing for a secret one.
fn field_table<'a>(fields: impl IntoIterator<Item = (&'a CredentialField, Option<&'a str>)>) -> Table {
    let mut table = Table::new(["name", "kind", "value", "label"]);
    for (field, value) in fields {
        table.row::<Cell>([
            field.name.as_str().into(),
            field.kind.as_str().into(),
            value.into(),
            field.label.as_str().into(),
        ]);
    }
    table
}

/// A credential as `list` lists it and `request` answers it.
fn listed_text(listed: &Listed) -> String {
    let mut fields = Fields::new();
    fields
        .field("Name", listed.name.as_str())
        .field("Keep until", listed.keep_until)
        .field("Allowed", listed.allowed)
        .table("Fields", &field_table(listed.fields.iter().map(|field| (field, None))));
    fields.render()
}

pub(crate) async fn print_json(port: &RpcPort, value: &impl Serialize) -> Result<u8, RpcError> {
    let text = serde_json::to_string(value).map_err(failed)?;
    port.stdout(format!("{text}\n")).await?;
    Ok(0)
}

async fn list(shard: Rc<Shard>, call: Call<NoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let credentials = shard
        .services()
        .control
        .user_credentials(shard.user().clone())
        .await
        .map_err(failed)?;
    let allowed = allowed(&shard, &call.invocation).await?;
    let listed: Vec<Listed> = credentials
        .iter()
        .map(|credential| listed(credential, allowed.contains(&credential.id)))
        .collect();
    if call.invocation.json {
        return print_json(&port, &listed).await;
    }
    let mut table = Table::new(["name", "fields", "keep until", "allowed"]);
    for credential in &listed {
        table.row::<Cell>([
            credential.name.as_str().into(),
            field_names(&credential.fields).into(),
            credential.keep_until.into(),
            credential.allowed.into(),
        ]);
    }
    port.stdout(table.render()).await?;
    Ok(0)
}

async fn show(shard: Rc<Shard>, call: Call<NameArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let control = &shard.services().control;
    let name = call.args.name;
    let Some(credential) = control
        .user_credential_named(shard.user().clone(), name.clone())
        .await
        .map_err(failed)?
    else {
        return Err(failed(format!(
            "\u{201c}{name}\u{201d}: the user has no credential of that name"
        )));
    };
    let allowed = allowed(&shard, &call.invocation)
        .await?
        .contains(&credential.id);
    // A visible value is shown once the conversation may use the
    // credential; a secret one never.
    let mut values = if allowed {
        control
            .user_credential_values(shard.user().clone(), credential.id.clone(), shard.vault_seal())
            .await
            .map_err(failed)?
            .map(|(_, values)| values)
            .unwrap_or_default()
    } else {
        Default::default()
    };
    let credential_fields = credential.fields.clone();
    let shown = Shown {
        fields: credential
            .fields
            .iter()
            .map(|field| ShownField {
                name: field.name.clone(),
                label: field.label.as_str().to_owned(),
                kind: field.kind,
                value: match field.kind {
                    FieldKind::Visible => values.remove(&field.name),
                    FieldKind::Secret => None,
                },
            })
            .collect(),
        name: credential.name,
        note: credential.note,
        keep_until: credential.keep_until,
        last_used_at: credential.last_used_at,
        created_at: credential.created_at,
        updated_at: credential.updated_at,
        allowed,
    };
    if call.invocation.json {
        return print_json(&port, &shown).await;
    }
    let mut fields = Fields::new();
    fields
        .field("Name", shown.name.as_str())
        .field("Keep until", shown.keep_until)
        .field("Last used", shown.last_used_at)
        .field("Created", shown.created_at)
        .field("Updated", shown.updated_at)
        .field("Allowed", shown.allowed)
        .table(
            "Fields",
            &field_table(credential_fields.iter().zip(&shown.fields).map(|(field, shown)| (field, shown.value.as_deref()))),
        )
        .text("Note", &shown.note);
    port.stdout(fields.render()).await?;
    Ok(0)
}

/// One `--field` value, `<name>:<secret|visible>:<label>`.
fn parse_field(text: &str) -> Result<CredentialField, RpcError> {
    let usage = || {
        failed(format!(
            "--field {text:?}: a field is <name>:<secret|visible>:<label>, such as token:secret:\"Access token\""
        ))
    };
    let (name, rest) = text.split_once(':').ok_or_else(usage)?;
    let (kind, label) = rest.split_once(':').ok_or_else(usage)?;
    let kind = match kind {
        "secret" => FieldKind::Secret,
        "visible" => FieldKind::Visible,
        _ => return Err(usage()),
    };
    let field = CredentialField {
        name: name.to_owned(),
        label: Trimmed::from(label.to_owned()),
        kind,
    };
    field
        .validate()
        .map_err(|report| failed(format!("--field {text:?}: {report}")))?;
    Ok(field)
}

async fn request(shard: Rc<Shard>, call: Call<RequestArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let RequestArgs {
        name,
        field,
        reason,
        keep,
    } = call.args;
    let name = Trimmed::from(name).into_string();
    let length = name.chars().count();
    if length == 0 || length > NAME_MAX {
        return Err(failed(format!(
            "a credential's name has 1 to {NAME_MAX} characters"
        )));
    }
    let fields = field
        .iter()
        .map(|text| parse_field(text))
        .collect::<Result<Vec<_>, _>>()?;
    distinct(fields.iter().map(|field| field.name.as_str()))
        .map_err(|error| failed(format!("--field: {error}")))?;
    let ask = CredentialAsk {
        reason: reason.map(|reason| Trimmed::from(reason).into_string()).unwrap_or_default(),
        fields,
        keep: keep.unwrap_or(Keep::Hour),
    };
    let answer = ask_credential(
        shard.permission_shard(),
        &call.invocation,
        name.clone(),
        ask,
        port.cancelled(),
    )
    .await?;
    match answer {
        Answer::Saved { credential, .. } => {
            let saved = listed(&credential, true);
            if call.invocation.json {
                return print_json(&port, &saved).await;
            }
            port.stdout(listed_text(&saved)).await?;
            Ok(0)
        }
        Answer::Declined => Err(failed(format!(
            "the user declined to save \u{201c}{name}\u{201d}"
        ))),
        Answer::Withdrawn => Err(failed(format!(
            "the request for \u{201c}{name}\u{201d} was withdrawn: the conversation was archived"
        ))),
    }
}

async fn read(shard: Rc<Shard>, call: Call<ReadArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let value = read_credential(
        shard.permission_shard(),
        &call.invocation,
        call.args.name,
        call.args.field,
    )
    .await?;
    port.stdout(value).await?;
    Ok(0)
}

async fn remove(shard: Rc<Shard>, call: Call<NameArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let name = call.args.name;
    let Some(credential) = shard
        .services()
        .control
        .user_credential_named(shard.user().clone(), name.clone())
        .await
        .map_err(failed)?
    else {
        return Err(failed(format!(
            "\u{201c}{name}\u{201d}: the user has no credential of that name"
        )));
    };
    // Named as it is saved, whatever the case it was asked for in.
    let name = credential.name;
    match shard.remove_credential(credential.id).await {
        Ok(()) => {}
        Err(CredentialError::Refused(_)) => {
            return Err(failed(format!(
                "\u{201c}{name}\u{201d}: the user has no credential of that name"
            )));
        }
        Err(CredentialError::Storage(error)) => return Err(failed(error)),
    }
    port.stdout(format!("Removed the credential \u{201c}{name}\u{201d}.\n"))
        .await?;
    Ok(0)
}
