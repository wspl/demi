//! `demi host expose` (`expose.md` § Commands): the `rpc` leaves over the
//! user's expose records, which the backend owns. `add` exposes a service on
//! the calling conversation's main Host, or on the Host `--host` names as
//! `demi host list` shows it; the other leaves reach every expose of the
//! user, whichever conversation created it.

use std::collections::HashMap;
use std::rc::{Rc, Weak};

use demi_shell::{Call, GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_web_api::exposes::{ExposeAddress, ExposeAnswer, Exposes};
use demi_web_api::ids::ExposeId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ExposeError;
use super::records::LIFETIME;
use crate::conversation::host_access::HostRole;
use crate::runner::host_commands::{NoArgs, conversation_of, named_host, reachable, verb};
use crate::shard::Shard;

const SUMMARY: &str = "Give a service on a host a public URL for one hour: add, list, renew, remove.";

const ADD_SUMMARY: &str = "Expose a service on a host under a fresh public URL for one hour: `demi host expose add <host:port|port> [--host <name|id>]`.";

const LIST_SUMMARY: &str = "Every expose of this user across devices, soonest expiry first.";

const RENEW_SUMMARY: &str = "Set an expose's expiry to one hour from now.";

const REMOVE_SUMMARY: &str = "Destroy an expose at once; its URL no longer works.";

const NOT_FOUND_OUTPUT: &str =
    "expose_not_found when the id is not this user's or has expired; writes the reason to stderr and exits non-zero";

/// The input of `demi host expose add`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AddArgs {
    /// host:port, or a bare port meaning 127.0.0.1
    address: String,
    /// Host name or device id from demi host list; the main host by default
    host: Option<String>,
}

/// The input of `demi host expose renew` and `remove`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ExposeArgs {
    /// Expose id
    id: String,
}

/// The `expose` group of `demi host`, whose handlers act in `shard`.
pub(crate) fn expose_group(shard: &Weak<Shard>) -> GroupBuilder {
    GroupBuilder::new("expose", SUMMARY)
        .leaf(
            LeafBuilder::rpc("add", ADD_SUMMARY)
                .input::<AddArgs>()
                .positionals(["address"])
                .json_output::<ExposeAnswer>()
                .success_output("the URL, the device, and the expiry, or JSON matching { expose } when --json is passed")
                .failure_output("expose_unavailable without an expose domain, an unreachable --host, an address that is not host:port or a port, or a device that is not connected; writes the reason to stderr and exits non-zero")
                .bind(TypedRpc::new(verb(shard.clone(), add))),
        )
        .leaf(
            LeafBuilder::rpc("list", LIST_SUMMARY)
                .input::<NoArgs>()
                .json_output::<Exposes>()
                .success_output("one line per expose under a header, or JSON matching { exposes } when --json is passed")
                .failure_output("never fails on live data")
                .bind(TypedRpc::new(verb(shard.clone(), list))),
        )
        .leaf(
            LeafBuilder::rpc("renew", RENEW_SUMMARY)
                .input::<ExposeArgs>()
                .positionals(["id"])
                .json_output::<ExposeAnswer>()
                .success_output("the new expiry, or JSON matching { expose } when --json is passed")
                .failure_output(NOT_FOUND_OUTPUT)
                .bind(TypedRpc::new(verb(shard.clone(), renew))),
        )
        .leaf(
            LeafBuilder::rpc("remove", REMOVE_SUMMARY)
                .input::<ExposeArgs>()
                .positionals(["id"])
                .success_output("confirms the removal")
                .failure_output(NOT_FOUND_OUTPUT)
                .bind(TypedRpc::new(verb(shard.clone(), remove))),
        )
}

async fn add(shard: Rc<Shard>, call: Call<AddArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let Call {
        args: AddArgs { address, host: wanted },
        invocation,
    } = call;
    let conversation = conversation_of(&invocation)?;
    let hosts = reachable(&shard, &conversation).await?;
    let target = match &wanted {
        Some(wanted) => named_host(&hosts, wanted),
        None => hosts.iter().find(|host| host.role == HostRole::Main),
    };
    let Some(target) = target else {
        let wanted = wanted.as_deref().unwrap_or("(main)");
        port.stderr(format!(
            "expose add: host {wanted} is not reachable from this conversation (see `demi host list`)\n"
        ))
        .await?;
        return Ok(1);
    };
    let address = match ExposeAddress::try_from(address) {
        Ok(address) => address,
        Err(error) => {
            port.stderr(format!("expose add: the address {error}\n")).await?;
            return Ok(1);
        }
    };
    let expose = match shard.add_expose(&target.device, address).await {
        Ok(expose) => expose,
        Err(error) => return refused(&port, "add", error).await,
    };
    let text = if invocation.json {
        json(&ExposeAnswer { expose })?
    } else {
        format!(
            "Exposed {} on {} as {}\nExpires in {} minutes (expose {}).\n",
            expose.address.as_str(),
            target.name,
            expose.url,
            LIFETIME.as_mins(),
            expose.id
        )
    };
    port.stdout(text).await?;
    Ok(0)
}

async fn list(shard: Rc<Shard>, call: Call<NoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let exposes = shard.list_exposes().await.map_err(failed)?;
    if call.invocation.json {
        port.stdout(json(&Exposes { exposes })?).await?;
        return Ok(0);
    }
    if exposes.is_empty() {
        port.stdout("No exposes.\n").await?;
        return Ok(0);
    }
    let mut names = HashMap::new();
    for expose in &exposes {
        if names.contains_key(&expose.device_id) {
            continue;
        }
        let device = shard
            .services()
            .control
            .device(expose.device_id.clone())
            .await
            .map_err(failed)?;
        let name = device.map_or_else(|| expose.device_id.to_string(), |device| device.name);
        names.insert(expose.device_id.clone(), name);
    }
    let now = shard.services().clock.now();
    let mut rows = vec![["Expose", "Device", "Address", "Expires", "URL"].map(str::to_owned)];
    for expose in &exposes {
        let left = expose.expires_at.as_millisecond().saturating_sub(now.as_millisecond()).max(0);
        rows.push([
            expose.id.to_string(),
            names[&expose.device_id].clone(),
            expose.address.as_str().to_owned(),
            format!("{} min", (left + 30_000) / 60_000),
            expose.url.clone(),
        ]);
    }
    port.stdout(table(&rows)).await?;
    Ok(0)
}

async fn renew(shard: Rc<Shard>, call: Call<ExposeArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let renewed = match ExposeId::try_from(call.args.id.as_str()) {
        Ok(id) => shard.renew_expose(&id).await,
        Err(_) => Err(ExposeError::NotFound(call.args.id)),
    };
    let expose = match renewed {
        Ok(expose) => expose,
        Err(error) => return refused(&port, "renew", error).await,
    };
    let text = if call.invocation.json {
        json(&ExposeAnswer { expose })?
    } else {
        format!("Expose {} expires in {} minutes.\n", expose.id, LIFETIME.as_mins())
    };
    port.stdout(text).await?;
    Ok(0)
}

async fn remove(shard: Rc<Shard>, call: Call<ExposeArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let removed = match ExposeId::try_from(call.args.id.as_str()) {
        Ok(id) => shard.remove_expose(&id).await.map(|()| id),
        Err(_) => Err(ExposeError::NotFound(call.args.id)),
    };
    match removed {
        Ok(id) => {
            port.stdout(format!("Removed expose {id}; its URL no longer works.\n")).await?;
            Ok(0)
        }
        Err(error) => refused(&port, "remove", error).await,
    }
}

/// A refusal as the leaf's stderr and exit code (`expose.md` § Commands); a
/// failure the call could not cause fails it.
async fn refused(port: &RpcPort, leaf: &str, error: ExposeError) -> Result<u8, RpcError> {
    let Some((code, _)) = error.code() else {
        return Err(failed(error));
    };
    port.stderr(format!("expose {leaf}: {error} ({code})\n")).await?;
    Ok(1)
}

fn failed(error: impl std::fmt::Display) -> RpcError {
    RpcError::Failed(error.to_string())
}

fn json(value: &impl Serialize) -> Result<String, RpcError> {
    serde_json::to_string(value).map_err(failed)
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
