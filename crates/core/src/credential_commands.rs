//! The product's `demi credentials` group.
//!
//! An agent can:
//!
//! - list the user's credentials and show one;
//! - ask the user for one and wait for the answer;
//! - save one, or new values of a saved one, from values piped on its
//!   standard input;
//! - read one field's value onto its standard output;
//! - remove one.
//!
//! No output of these commands carries a secret value but `read`'s, which the
//! agent passes on.
//!
//! # Passing a secret on
//!
//! An agent that needs a secret asks the user for it with `request` rather than
//! reading it from the message, where it would sit in the transcript and reach
//! the model's vendor. One it can reach on the Host, such as a key in a file,
//! it pipes into `save`: never into an argument, since the shell expands it
//! there and the permission card shows and stores the expanded command line.
//! A save, a replacement of a saved one's values included, asks nothing
//! beyond the group's grant: an agent can already use any value it can pipe.
//!
//! `read` writes the value to its standard output, wherever that goes: Demi
//! refuses and masks nothing, and how the value is passed on is the agent's
//! choice. A field's kind is advice, not a rule: the command's help recommends
//! passing a secret where the model does not read it, into a pipe, a file or a
//! `$(…)`, since a value the job prints joins the transcript.
//!
//! The command line the model wrote, which the transcript keeps, holds the
//! `$(…)` as written, never its value. Demi writes no value into the transcript,
//! a log or a frame to the page, but a visible field's in Settings' list. A
//! value enters the transcript only when an agent's command prints it.
//!
//! # Permission and keeping
//!
//! Every command of the group needs the grant of the user's credentials
//! ([`CREDENTIALS`]), which the user gives once for a conversation and which
//! then covers every credential, for every agent of its tree.
//!
//! How long a credential is kept is the agent's decision, with the request's
//! `--keep`, since it knows what the secret is for. The card offers no choice,
//! as a form does not ask how long its data is kept.

use std::collections::BTreeMap;
use std::rc::{Rc, Weak};

use demi_core_store::credentials::StoredCredential;
use demi_core_store::permissions::CredentialAsk;
use demi_core_permissions::{Answer, ask_credential, read_credential, save_credential};
use demi_host_interface::{
    Call, GroupBuilder, LeafBuilder, RpcError, RpcInvocation, RpcPort, TypedRpc,
};
use demi_shared_types::{
    Nullable, Timestamp,
    output::{Cell, Fields, Table},
};
use demi_web_api_protocol::credentials::{
    CredentialField, FieldKind, Keep, distinct,
};
use demi_web_api_protocol::text::Trimmed;
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Core;
use super::credentials::CredentialError;
use crate::conversation::organize::{failed, verb};

/// The permission category every command of the group needs.
pub(crate) const CREDENTIALS: &str = "credentials.use";

const CREDENTIALS_ACTION: &str = "use your credentials";

const CREDENTIALS_DESCRIPTION: &str = "Use Your Credentials lets the agents of this conversation list, read, save, replace and remove your saved credentials, and ask you for new ones.";

const SUMMARY: &str = "The user's credentials: named secrets such as API keys, tokens and passwords, each with fields. Ask the user for one, pass a field's value on without reading it, list and show them, or remove one. Every command needs the user's permission once for the conversation.";

/// The group's entry in the model's capability index.
const ENTRY: &str = "The user's named secrets: request one on a card, or save one piped from the Host; read a field into a pipe or a file. A secret is asked for this way, never in the conversation. A reply shows one to the user with `<credential name=\"…\" />` on its own line, `field=\"…\"` naming one of several.";

const LIST_SUMMARY: &str = "List the user's credentials: name, field names and when each is removed (- for kept always). No value.";

const SHOW_SUMMARY: &str = "Show one credential: its fields, the values of its visible fields, its note and its dates. A secret field's value is never shown; `read` passes it on.";

const REQUEST_SUMMARY: &str = "Ask the user for a credential on a card in the conversation, or for new values of the saved one of that name, which then must have the same fields. Waits for the user's answer, however long, and prints what was saved, or fails when the user declines.";

const SAVE_SUMMARY: &str = "Save a credential from values piped on standard input, which the model never reads, such as a key from a file on the Host, or new values of the saved one of that name, which then must have the same fields.";

const VALUES: &str = "Piped from a file or a command, never written in the command, which the transcript keeps: for one field, its value as it is, without one trailing newline; for several, one <field>=<value> line each.";

const READ_SUMMARY: &str = "Write one field's value to standard output, exactly as saved, without a newline. A value you print is sent to the model's vendor and stays in the transcript; to avoid that, pipe it into a command (`demi credentials read \"GitHub token\" token | gh auth login --with-token`), redirect it into a file (`… > token.txt`), or substitute it (`TOKEN=$(demi credentials read \"GitHub token\" token) git push`).";

const REMOVE_SUMMARY: &str = "Remove a credential and its values. Only when the user asks for it.";

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
    #[schemars(length(min = 1))]
    field: Vec<String>,
    reason: Option<String>,
    keep: Option<Keep>,
}

/// The input of `save`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SaveArgs {
    name: String,
    #[schemars(length(min = 1))]
    field: Vec<String>,
    reason: Option<String>,
    keep: Option<Keep>,
    /// The values, from standard input.
    values: Option<String>,
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
}

/// A field as `show` prints it: a visible field's value, never a secret
/// one's.
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
}

/// The `credentials` group, whose handlers act in `core`.
pub(crate) fn credentials_group(core: Weak<Core>) -> GroupBuilder {
    GroupBuilder::new("credentials", SUMMARY)
        .index_entry(ENTRY)
        .permission(CREDENTIALS, CREDENTIALS_ACTION, CREDENTIALS_DESCRIPTION)
        .leaf(
            LeafBuilder::rpc("list", LIST_SUMMARY)
                .input::<NoArgs>()
                .json_output::<Vec<Listed>>()
                .permission(CREDENTIALS)
                .bind(TypedRpc::new(verb(core.clone(), list))),
        )
        .leaf(
            LeafBuilder::rpc("show", SHOW_SUMMARY)
                .input::<NameArgs>()
                .describe("name", "The credential's name.")
                .positionals(["name"])
                .json_output::<Shown>()
                .permission(CREDENTIALS)
                .bind(TypedRpc::new(verb(core.clone(), show))),
        )
        .leaf(
            LeafBuilder::rpc("request", REQUEST_SUMMARY)
                .input::<RequestArgs>()
                .describe(
                    "name",
                    "The credential's name, such as \"GitHub token for deploys\".",
                )
                .describe(
                    "field",
                    "One field as <name>:<secret|visible>:<label>, such as token:secret:\"Personal access token\" or user:visible:\"User name\". A name is lowercase letters, digits, _ and -. A secret field is typed hidden and never listed; a visible one, such as a user name or a host, is shown as it is.",
                )
                .describe("reason", "Why you ask, which the card shows the user and the credential keeps as its note.")
                .describe("keep", "How long the credential is kept, which you decide by what it is for and the card tells the user: 1h (the default) for a secret used once, such as a key to add a provider; 1d or 30d for work that goes on that long; always for one the user wants you to keep using, which stays in Settings › Credentials.")
                .positionals(["name"])
                .running_hint(RUNNING_HINT)
                .json_output::<Listed>()
                .permission(CREDENTIALS)
                .bind(TypedRpc::new(verb(core.clone(), request))),
        )
        .leaf(
            LeafBuilder::rpc("save", SAVE_SUMMARY)
                .input::<SaveArgs>()
                .describe("name", "The credential's name.")
                .describe("field", "One field as <name>:<secret|visible>:<label>, as `request` takes it.")
                .describe("reason", "What it is for, which the credential keeps as its note.")
                .describe("keep", "How long the credential is kept, as for `request`: 1h (the default), 1d, 30d or always.")
                .describe("values", VALUES)
                .positionals(["name"])
                .stdin_field("values")
                .json_output::<Listed>()
                .permission(CREDENTIALS)
                .bind(TypedRpc::new(verb(core.clone(), save))),
        )
        .leaf(
            LeafBuilder::rpc("read", READ_SUMMARY)
                .input::<ReadArgs>()
                .describe("name", "The credential's name.")
                .describe("field", "The field's name.")
                .positionals(["name", "field"])
                .permission(CREDENTIALS)
                .bind(TypedRpc::new(verb(core.clone(), read))),
        )
        .leaf(
            LeafBuilder::rpc("remove", REMOVE_SUMMARY)
                .input::<NameArgs>()
                .describe("name", "The credential's name.")
                .positionals(["name"])
                .permission(CREDENTIALS)
                .bind(TypedRpc::new(verb(core, remove))),
        )
}

fn listed(credential: &StoredCredential) -> Listed {
    Listed {
        name: credential.name.clone(),
        fields: credential.fields.clone(),
        keep_until: credential.keep_until,
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
        .table("Fields", &field_table(listed.fields.iter().map(|field| (field, None))));
    fields.render()
}

pub(crate) async fn print_json(port: &RpcPort, value: &impl Serialize) -> Result<u8, RpcError> {
    let text = serde_json::to_string(value).map_err(failed)?;
    port.stdout(format!("{text}\n")).await?;
    Ok(0)
}

async fn list(core: Rc<Core>, call: Call<NoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let credentials = core
        .store().user_credentials()
        .await
        .map_err(failed)?;
    let listed: Vec<Listed> = credentials.iter().map(listed).collect();
    if call.invocation.json {
        return print_json(&port, &listed).await;
    }
    let mut table = Table::new(["name", "fields", "keep until"]);
    for credential in &listed {
        table.row::<Cell>([
            credential.name.as_str().into(),
            field_names(&credential.fields).into(),
            credential.keep_until.into(),
        ]);
    }
    port.stdout(table.render()).await?;
    Ok(0)
}

async fn show(core: Rc<Core>, call: Call<NameArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let store = core.store();
    let name = call.args.name;
    let Some(credential) = store.user_credential_named(name.clone())
        .await
        .map_err(failed)?
    else {
        return Err(failed(format!(
            "\u{201c}{name}\u{201d}: the user has no credential of that name"
        )));
    };
    // A visible value is shown; a secret one never.
    let mut values = store.user_credential_values(credential.id.clone(), core.vault_seal())
        .await
        .map_err(failed)?
        .map(|(_, values)| values)
        .unwrap_or_default();
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

/// The name and the ask a `request` or a `save` names.
fn asked(
    name: String,
    field: &[String],
    reason: Option<String>,
    keep: Option<Keep>,
) -> Result<(String, CredentialAsk), RpcError> {
    let name = Trimmed::from(name).into_string();
    if name.is_empty() {
        return Err(failed("a credential's name has at least one character".to_owned()));
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
    Ok((name, ask))
}

/// Each field's value from `save`'s standard input. A problem names the
/// line, never what it holds, which may be a secret.
fn piped_values(fields: &[CredentialField], input: &str) -> Result<BTreeMap<String, String>, RpcError> {
    let mut values = BTreeMap::new();
    if let [field] = fields {
        let value = input
            .strip_suffix("\r\n")
            .or_else(|| input.strip_suffix('\n'))
            .unwrap_or(input);
        values.insert(field.name.clone(), value.to_owned());
    } else {
        for (index, line) in input.lines().enumerate() {
            if line.is_empty() {
                continue;
            }
            let line_number = index + 1;
            let (name, value) = line.split_once('=').ok_or_else(|| {
                failed(format!("standard input, line {line_number}: a line is <field>=<value>"))
            })?;
            if !fields.iter().any(|field| field.name == name) {
                return Err(failed(format!(
                    "standard input, line {line_number}: names no field of the credential; its fields are {}",
                    field_names(fields)
                )));
            }
            if values.insert(name.to_owned(), value.to_owned()).is_some() {
                return Err(failed(format!(
                    "standard input, line {line_number}: \u{201c}{name}\u{201d} has a value already"
                )));
            }
        }
    }
    for field in fields {
        match values.get(&field.name) {
            None => {
                return Err(failed(format!(
                    "standard input has no value for \u{201c}{}\u{201d}; pipe one <field>=<value> line for each field",
                    field.name
                )));
            }
            Some(value) if value.is_empty() => {
                return Err(failed(format!(
                    "standard input: the value of \u{201c}{}\u{201d} is empty",
                    field.name
                )));
            }
            Some(_) => {}
        }
    }
    Ok(values)
}

async fn request(core: Rc<Core>, call: Call<RequestArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let RequestArgs {
        name,
        field,
        reason,
        keep,
    } = call.args;
    let (name, ask) = asked(name, &field, reason, keep)?;
    let answer = ask_credential(
        core.permission_core(),
        &call.invocation,
        name.clone(),
        ask,
        port.cancelled(),
    )
    .await?;
    match answer {
        Answer::Saved { credential, .. } => print_saved(&call.invocation, &port, &credential).await,
        Answer::Declined => Err(failed(format!(
            "the user declined to save \u{201c}{name}\u{201d}"
        ))),
        Answer::Withdrawn => Err(failed(format!(
            "the request for \u{201c}{name}\u{201d} was withdrawn: the conversation was archived"
        ))),
    }
}

async fn save(core: Rc<Core>, call: Call<SaveArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let SaveArgs {
        name,
        field,
        reason,
        keep,
        values,
    } = call.args;
    let (name, ask) = asked(name, &field, reason, keep)?;
    let values = values.ok_or_else(|| failed("pipe the values on standard input".to_owned()))?;
    let values = piped_values(&ask.fields, &values)?;
    let saved = save_credential(core.permission_core(), &call.invocation, name, ask, values).await?;
    print_saved(&call.invocation, &port, &saved.credential).await
}

/// Prints the credential `request` or `save` saved.
async fn print_saved(invocation: &RpcInvocation, port: &RpcPort, credential: &StoredCredential) -> Result<u8, RpcError> {
    let saved = listed(credential);
    if invocation.json {
        return print_json(port, &saved).await;
    }
    port.stdout(listed_text(&saved)).await?;
    Ok(0)
}

async fn read(core: Rc<Core>, call: Call<ReadArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let value = read_credential(
        core.permission_core(),
        &call.invocation,
        call.args.name,
        call.args.field,
    )
    .await?;
    port.stdout(value).await?;
    Ok(0)
}

async fn remove(core: Rc<Core>, call: Call<NameArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let name = call.args.name;
    let Some(credential) = core
        .store().user_credential_named(name.clone())
        .await
        .map_err(failed)?
    else {
        return Err(failed(format!(
            "\u{201c}{name}\u{201d}: the user has no credential of that name"
        )));
    };
    // Named as it is saved, whatever the case it was asked for in.
    let name = credential.name;
    match core.remove_credential(credential.id).await {
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
