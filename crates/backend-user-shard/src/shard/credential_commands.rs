//! The product's `demi credentials` group (`credentials.md` § Commands): an
//! agent lists the user's credentials and shows one, asks the user for one
//! and waits for the answer, reads one field's value onto its standard
//! output once the conversation may use the credential, and, with the
//! user's Manage Credentials, removes one. No output of these commands
//! carries a secret value but `read`'s, which the agent passes on.

use std::collections::BTreeSet;
use std::rc::{Rc, Weak};

use demi_backend_database::credentials::StoredCredential;
use demi_backend_database::permissions::CredentialAsk;
use demi_backend_permissions::{Answer, ask_credential, read_credential};
use demi_backend_host_access::host_commands::conversation_of;
use demi_host_interface::{
    Call, GroupBuilder, LeafBuilder, RpcError, RpcInvocation, RpcPort, TypedRpc,
};
use demi_shared_types::{Nullable, Timestamp};
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

/// The group's entry in the model's capability index (`system-prompt.md`
/// § Capability index).
const ENTRY: &str = "Keeps the secrets a task needs, API keys, tokens, passwords, as the user's named credentials: `request` asks the user for one on a card and waits for the answer; `read` writes one field's value to standard output, to pass into a pipe, a file or `$(…)` without printing it. Use it whenever a task needs a secret, rather than asking the user to paste it into the conversation, where it would join the transcript.";

const LIST_SUMMARY: &str = "List the user's credentials: name, fields (name, kind, label), when each is removed, and whether this conversation may use it. No value.";

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

/// `list --json`.
#[derive(Serialize, JsonSchema)]
struct Listing {
    credentials: Vec<Listed>,
}

/// `request --json`.
#[derive(Serialize, JsonSchema)]
struct Saved {
    saved: Listed,
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

/// `show --json`.
#[derive(Serialize, JsonSchema)]
struct ShowAnswer {
    credential: Shown,
}

/// The `credentials` group, whose handlers act in `shard`.
pub(crate) fn credentials_group(shard: Weak<Shard>) -> GroupBuilder {
    GroupBuilder::new("credentials", SUMMARY)
        .index_entry(ENTRY)
        .permission(MANAGE, MANAGE_ACTION, MANAGE_DESCRIPTION)
        .leaf(
            LeafBuilder::rpc("list", LIST_SUMMARY)
                .input::<NoArgs>()
                .json_output::<Listing>()
                .bind(TypedRpc::new(verb(shard.clone(), list))),
        )
        .leaf(
            LeafBuilder::rpc("show", SHOW_SUMMARY)
                .input::<NameArgs>()
                .describe("name", "The credential's name.")
                .positionals(["name"])
                .json_output::<ShowAnswer>()
                .bind(TypedRpc::new(verb(shard.clone(), show))),
        )
        .leaf(
            LeafBuilder::rpc("request", REQUEST_SUMMARY)
                .input::<RequestArgs>()
                .describe(
                    "name",
                    format!("The credential's name, 1 to {NAME_MAX} characters, such as \"GitHub token for zan\"."),
                )
                .describe(
                    "field",
                    "One field as <name>:<secret|visible>:<label>, such as token:secret:\"Personal access token\" or user:visible:\"User name\"; one to ten. A name is lowercase letters, digits, _ and -. A secret field is typed hidden and never listed; a visible one, such as a user name or a host, is shown as it is.",
                )
                .describe("reason", "Why you ask, which the card shows the user and the credential keeps as its note.")
                .describe("keep", "How long the card offers to keep it at first: 1h (the default), 1d, 30d or always; the user decides.")
                .positionals(["name"])
                .running_hint(RUNNING_HINT)
                .json_output::<Saved>()
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

/// A credential's fields in one line: `key (secret: API key), user (visible:
/// User name)`.
fn field_line(fields: &[CredentialField]) -> String {
    fields
        .iter()
        .map(|field| format!("{} ({}: {})", field.name, field.kind.as_str(), field.label.as_str()))
        .collect::<Vec<_>>()
        .join(", ")
}

fn kept(keep_until: Option<Timestamp>) -> String {
    match keep_until {
        Some(time) => format!("until {time}"),
        None => "always".to_owned(),
    }
}

async fn print_json(port: &RpcPort, value: &impl Serialize) -> Result<u8, RpcError> {
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
        return print_json(&port, &Listing { credentials: listed }).await;
    }
    if listed.is_empty() {
        port.stdout("The user has no credentials.\n").await?;
        return Ok(0);
    }
    let mut text = String::new();
    for credential in &listed {
        let use_here = if credential.allowed {
            "this conversation may use it"
        } else {
            "this conversation must ask first, which `read` does"
        };
        text.push_str(&format!(
            "{}\n  fields: {}\n  kept: {}; {use_here}\n",
            credential.name,
            field_line(&credential.fields),
            kept(credential.keep_until),
        ));
    }
    port.stdout(text).await?;
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
        return print_json(&port, &ShowAnswer { credential: shown }).await;
    }
    let mut text = format!("{}\n", shown.name);
    for field in &shown.fields {
        let value = match (field.kind, &field.value) {
            (FieldKind::Secret, _) => "(secret; `read` passes it on)".to_owned(),
            (FieldKind::Visible, Some(value)) => value.clone(),
            (FieldKind::Visible, None) => "(shown once this conversation may use it)".to_owned(),
        };
        text.push_str(&format!(
            "  {} ({}: {}): {value}\n",
            field.name,
            field.kind.as_str(),
            field.label
        ));
    }
    if !shown.note.is_empty() {
        text.push_str(&format!("note: {}\n", shown.note));
    }
    text.push_str(&format!("kept: {}\n", kept(shown.keep_until)));
    text.push_str(&format!(
        "last used: {}\n",
        shown
            .last_used_at
            .map_or_else(|| "never".to_owned(), |time| time.to_string())
    ));
    text.push_str(&format!("created: {}; changed: {}\n", shown.created_at, shown.updated_at));
    text.push_str(if shown.allowed {
        "this conversation may use it\n"
    } else {
        "this conversation must ask first, which `read` does\n"
    });
    port.stdout(text).await?;
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
        .map_err(|error| failed(format!("--field {error}")))?;
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
        Answer::Saved { credential, keep } => {
            if call.invocation.json {
                return print_json(&port, &Saved { saved: listed(&credential, true) }).await;
            }
            let fields = credential
                .fields
                .iter()
                .map(|field| format!("{}: {}", field.name, field.kind.as_str()))
                .collect::<Vec<_>>()
                .join(", ");
            port.stdout(format!(
                "Saved credential \u{201c}{}\u{201d} ({fields}), {}.\n",
                credential.name,
                keep.phrase()
            ))
            .await?;
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
