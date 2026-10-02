//! `demi expose` (`expose.md` § Commands): the plugin's `rpc` leaves over
//! the user's exposes. `add` exposes a service on the calling
//! conversation's main Host, or on the Host `--host` names as `demi host
//! list` shows it; the other leaves reach every expose of the user,
//! whichever conversation created it, by its number.

use demi_host_interface::{GroupBuilder, LeafBuilder, RpcInvocation};
use demi_plugin_interface::{
    CommandPlugin, ExposeRefusal, HostRole, Placement, PluginError, PluginPort, PortFailure,
    PortHandled, PortRefusal,
};
use demi_shared_types::Timestamp;
use schemars::JsonSchema;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::LIFETIME;
use crate::numbers::{Numbered, numbered};

const SUMMARY: &str =
    "Give a service on a host a public URL for one hour: add, list, renew, remove.";

const ADD_SUMMARY: &str = "Expose a service on a host under a fresh public URL for one hour: `demi expose add <host:port|port> [--host <name|id>]`.";

const LIST_SUMMARY: &str = "Every expose of this user across devices, soonest expiry first.";

const RENEW_SUMMARY: &str = "Set an expose's expiry to one hour from now.";

const REMOVE_SUMMARY: &str = "Destroy an expose at once; its URL no longer works.";

const NOT_FOUND_OUTPUT: &str = "\"no expose <number>\" when the number names none of this user's live exposes; writes the reason to stderr and exits non-zero";

/// The input of `demi expose add`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddArgs {
    /// host:port, or a bare port meaning 127.0.0.1
    address: String,
    /// Host name or device id from demi host list; the main host by default
    host: Option<String>,
}

/// The input of `demi expose list`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ListArgs {}

/// The input of `demi expose renew` and `remove`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NumberArgs {
    /// Expose number, as add and list print it
    number: u64,
}

/// An expose as the commands print it: by its number, never by its id,
/// which is the URL's credential.
#[derive(Debug, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ExposeLine {
    pub number: u64,
    /// The device's name.
    pub device: String,
    pub address: String,
    pub url: String,
    pub expires_at: Timestamp,
}

/// `{ expose }`: what `add` and `renew` print with `--json`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ExposeAnswer {
    pub expose: ExposeLine,
}

/// `{ exposes }`: what `list` prints with `--json`.
#[derive(Debug, Serialize, JsonSchema)]
pub struct ExposeLines {
    pub exposes: Vec<ExposeLine>,
}

/// The `expose` group, whose leaves the plugin answers with its port.
pub(crate) fn commands() -> CommandPlugin {
    let group = GroupBuilder::new("expose", SUMMARY)
        .leaf(
            LeafBuilder::rpc("add", ADD_SUMMARY)
                .input::<AddArgs>()
                .positionals(["address"])
                .json_output::<ExposeAnswer>()
                .success_output("the URL, the device, and the expiry, or JSON matching { expose } when --json is passed")
                .failure_output("exposes that are not available on this instance, an unreachable --host, an address that is not host:port or a port, or a device that is not connected; writes the reason to stderr and exits non-zero")
                .bind(PortHandled),
        )
        .leaf(
            LeafBuilder::rpc("list", LIST_SUMMARY)
                .input::<ListArgs>()
                .json_output::<ExposeLines>()
                .success_output("one line per expose under a header, or JSON matching { exposes } when --json is passed")
                .failure_output("never fails on live data")
                .bind(PortHandled),
        )
        .leaf(
            LeafBuilder::rpc("renew", RENEW_SUMMARY)
                .input::<NumberArgs>()
                .positionals(["number"])
                .json_output::<ExposeAnswer>()
                .success_output("the new expiry, or JSON matching { expose } when --json is passed")
                .failure_output(NOT_FOUND_OUTPUT)
                .bind(PortHandled),
        )
        .leaf(
            LeafBuilder::rpc("remove", REMOVE_SUMMARY)
                .input::<NumberArgs>()
                .positionals(["number"])
                .success_output("confirms the removal")
                .failure_output(NOT_FOUND_OUTPUT)
                .bind(PortHandled),
        );
    CommandPlugin::new(Placement::Demi, vec![group])
        .expect("the expose group is a valid declaration")
}

/// Runs the leaf `invocation` names, which [`CommandPlugin::check`]
/// checked, and answers its exit status.
pub(crate) async fn run(invocation: RpcInvocation, port: &PluginPort) -> Result<u8, PluginError> {
    let leaf = invocation.path.last().map(String::as_str);
    let reply = match leaf {
        Some("add") => add(args(&invocation)?, &invocation, port).await?,
        Some("list") => list(&invocation, port).await?,
        Some("renew") => renew(args(&invocation)?, &invocation, port).await?,
        Some("remove") => remove(args(&invocation)?, port).await?,
        _ => return Err(PluginError::failed("no such expose command")),
    };
    let rpc = port.rpc();
    match reply {
        Ok(text) => {
            rpc.stdout(text).await?;
            Ok(0)
        }
        Err(refusal) => {
            let leaf = leaf.unwrap_or_default();
            rpc.stderr(format!("expose {leaf}: {refusal}\n")).await?;
            Ok(1)
        }
    }
}

/// What a leaf prints: its output, or why it refused.
type Printed = Result<String, String>;

async fn add(
    args: AddArgs,
    invocation: &RpcInvocation,
    port: &PluginPort,
) -> Result<Printed, PluginError> {
    let hosts = port.conversation_hosts().await?;
    let target = match &args.host {
        Some(wanted) => hosts
            .iter()
            .find(|host| host.name == *wanted)
            .or_else(|| hosts.iter().find(|host| host.device.as_str() == wanted)),
        None => hosts.iter().find(|host| host.role == HostRole::Main),
    };
    let Some(target) = target else {
        let wanted = args.host.as_deref().unwrap_or("(main)");
        return Ok(Err(format!(
            "host {wanted} is not reachable from this conversation (see `demi host list`)"
        )));
    };
    let created = port
        .create_expose(target.device.clone(), args.address, LIFETIME)
        .await;
    let created = match refusal(created)? {
        Ok(created) => created,
        Err(refused) => return Ok(Err(refused)),
    };
    let numbered = numbered(port, port.exposes().await?.exposes).await?;
    let Some(entry) = numbered
        .into_iter()
        .find(|entry| entry.expose.id == created.id)
    else {
        // It ended before it could be numbered, as a Cloud's stop ends it.
        return Ok(Err("the expose ended at once".into()));
    };
    if invocation.json {
        return json(&ExposeAnswer {
            expose: line(&entry),
        })
        .map(Ok);
    }
    Ok(Ok(format!(
        "Exposed {} on {} as {}\nExpires in {} minutes (expose {}).\n",
        entry.expose.address.as_str(),
        target.name,
        entry.expose.url,
        LIFETIME / 60,
        entry.number
    )))
}

async fn list(invocation: &RpcInvocation, port: &PluginPort) -> Result<Printed, PluginError> {
    let listed = port.exposes().await?;
    let now = listed.listed_at;
    let exposes = numbered(port, listed.exposes).await?;
    if invocation.json {
        let exposes = exposes.iter().map(line).collect();
        return json(&ExposeLines { exposes }).map(Ok);
    }
    if exposes.is_empty() {
        return Ok(Ok("No exposes.\n".into()));
    }
    let mut rows = vec![["Expose", "Device", "Address", "Expires", "URL"].map(str::to_owned)];
    for entry in &exposes {
        let left = entry
            .expose
            .expires_at
            .as_millisecond()
            .saturating_sub(now.as_millisecond())
            .max(0);
        rows.push([
            entry.number.to_string(),
            entry.expose.device_name.clone(),
            entry.expose.address.as_str().to_owned(),
            format!("{} min", (left + 30_000) / 60_000),
            entry.expose.url.clone(),
        ]);
    }
    Ok(Ok(table(&rows)))
}

async fn renew(
    args: NumberArgs,
    invocation: &RpcInvocation,
    port: &PluginPort,
) -> Result<Printed, PluginError> {
    let Some(entry) = by_number(port, args.number).await? else {
        return Ok(Err(no_expose(args.number)));
    };
    let renewed = port.renew_expose(entry.expose.id.clone(), LIFETIME).await;
    let renewed = match refusal(renewed)? {
        Ok(renewed) => renewed,
        Err(_) => return Ok(Err(no_expose(args.number))),
    };
    if invocation.json {
        let entry = Numbered {
            number: entry.number,
            expose: renewed,
        };
        return json(&ExposeAnswer {
            expose: line(&entry),
        })
        .map(Ok);
    }
    Ok(Ok(format!(
        "Expose {} expires in {} minutes.\n",
        args.number,
        LIFETIME / 60
    )))
}

async fn remove(args: NumberArgs, port: &PluginPort) -> Result<Printed, PluginError> {
    let Some(entry) = by_number(port, args.number).await? else {
        return Ok(Err(no_expose(args.number)));
    };
    match refusal(port.remove_expose(entry.expose.id).await)? {
        Ok(()) => Ok(Ok(format!(
            "Removed expose {}; its URL no longer works.\n",
            args.number
        ))),
        Err(_) => Ok(Err(no_expose(args.number))),
    }
}

/// The user's live expose of `number`.
async fn by_number(port: &PluginPort, number: u64) -> Result<Option<Numbered>, PluginError> {
    let exposes = numbered(port, port.exposes().await?.exposes).await?;
    Ok(exposes.into_iter().find(|entry| entry.number == number))
}

fn no_expose(number: u64) -> String {
    format!("no expose {number}")
}

/// An expose operation's answer, its refusal in the words the commands
/// print, or the port's failure.
fn refusal<T>(answer: Result<T, PortFailure>) -> Result<Result<T, String>, PluginError> {
    match answer {
        Ok(value) => Ok(Ok(value)),
        Err(PortFailure::Refused(PortRefusal::Expose { reason, message })) => {
            let words = match reason {
                ExposeRefusal::Unavailable => "exposes are not available on this instance".into(),
                ExposeRefusal::DeviceOffline => {
                    "the device is offline; connect it before exposing a service".into()
                }
                ExposeRefusal::InvalidAddress
                | ExposeRefusal::DeviceNotFound
                | ExposeRefusal::NotFound => message,
            };
            Ok(Err(words))
        }
        Err(failure) => Err(failure.into()),
    }
}

fn line(entry: &Numbered) -> ExposeLine {
    ExposeLine {
        number: entry.number,
        device: entry.expose.device_name.clone(),
        address: entry.expose.address.as_str().to_owned(),
        url: entry.expose.url.clone(),
        expires_at: entry.expose.expires_at,
    }
}

fn args<A: DeserializeOwned>(invocation: &RpcInvocation) -> Result<A, PluginError> {
    serde_json::from_value(Value::Object(invocation.args.clone())).map_err(|error| {
        PluginError::failed(format!(
            "the arguments of \"{}\" do not decode as declared: {error}",
            invocation.path.join(" ")
        ))
    })
}

fn json(value: &impl Serialize) -> Result<String, PluginError> {
    serde_json::to_string(value).map_err(PluginError::failed)
}

/// `rows` as lines of columns, each column as wide as its widest cell and
/// two spaces from the next; the last column is not padded.
fn table<const N: usize>(rows: &[[String; N]]) -> String {
    let mut widths = [0; N];
    for row in rows {
        for (width, cell) in widths.iter_mut().zip(row) {
            *width = (*width).max(cell.chars().count());
        }
    }
    let mut text = String::new();
    for row in rows {
        let cells: Vec<String> = row
            .iter()
            .zip(widths)
            .enumerate()
            .map(|(column, (cell, width))| {
                if column + 1 == N {
                    cell.clone()
                } else {
                    format!("{cell:<width$}")
                }
            })
            .collect();
        text.push_str(&cells.join("  "));
        text.push('\n');
    }
    text
}
