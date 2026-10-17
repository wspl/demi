//! The product's `demi notify` group (`turns.md` § The agent's
//! notifications): `send` sends the user a notification from the
//! conversation, at most 3 a minute and 30 an hour; `status` says whether
//! one would reach the user. Neither needs a permission, and no command
//! changes which events notify, which each browser chooses.

use std::rc::{Rc, Weak};

use demi_backend_host_access::host_commands::conversation_of;
use demi_host_interface::{Call, GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_shared_types::output::{Fields, warning};
use demi_web_api_protocol::push::PushMessage;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Shard;
use crate::conversation::organize::{failed, verb};

/// The longest title, and the longest body, in characters.
const TITLE_MAX: usize = 80;
const BODY_MAX: usize = 200;

const SUMMARY: &str = "Notifications to the user: send one from this conversation, which reaches the user's browsers and phone even with Demi closed, or check whether one would reach the user.";

/// The group's entry in the model's capability index (`system-prompt.md`
/// § Capability index).
const ENTRY: &str = "Sends the user a notification from this conversation, which reaches their browsers and phone even with every Demi page closed, and tells whether one would reach them. Use it when the user should know something while away, such as a long task that finished or needs them, and check `status` before relying on it. Not for the end of a turn the user watches, which needs none.";

const SEND_SUMMARY: &str = "Send the user a notification: its title and body show in the system's notifications, and a click opens this conversation. At most 3 a minute and 30 an hour per conversation.";

const STATUS_SUMMARY: &str = "Say whether a notification would reach the user: how many of the user's browsers have notifications on, and how many of them take the agent's notifications, with a warning on stderr when none does. The user chooses this in Settings › Notifications; no command changes it.";

/// The input of `send`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SendArgs {
    #[schemars(length(min = 1, max = TITLE_MAX))]
    title: String,
    #[schemars(length(max = BODY_MAX))]
    body: Option<String>,
}

/// The input of `status`, which takes none.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

/// `status --json`.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct Status {
    /// Whether a notification would reach the user: some browser takes the
    /// agent's notifications.
    reachable: bool,
    /// The user's browsers with notifications on.
    browsers: usize,
    /// Those of them that take the agent's notifications.
    agent_notifies: usize,
}

/// The `notify` group, whose handlers act in `shard`.
pub(crate) fn notify_group(shard: Weak<Shard>) -> GroupBuilder {
    GroupBuilder::new("notify", SUMMARY)
        .index_entry(ENTRY)
        .leaf(
            LeafBuilder::rpc("send", SEND_SUMMARY)
                .input::<SendArgs>()
                .describe("title", format!("The notification's title, 1 to {TITLE_MAX} characters, such as \"Deploy finished\"."))
                .describe("body", format!("What it says under the title, at most {BODY_MAX} characters."))
                .positionals(["title", "body"])
                .bind(TypedRpc::new(verb(shard.clone(), send))),
        )
        .leaf(
            LeafBuilder::rpc("status", STATUS_SUMMARY)
                .input::<NoArgs>()
                .json_output::<Status>()
                .bind(TypedRpc::new(verb(shard, status))),
        )
}

async fn send(shard: Rc<Shard>, call: Call<SendArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    let SendArgs { title, body } = call.args;
    let title = title.trim().to_owned();
    if title.is_empty() {
        return Err(failed("the title is blank"));
    }
    let now = shard.services().clock.now();
    shard
        .notifications()
        .take(&conversation, now)
        .map_err(failed)?;
    let message = PushMessage::Agent {
        conversation_id: conversation.clone(),
        id: uuid::Uuid::new_v4().to_string(),
        title,
        body: body.map(|body| body.trim().to_owned()).unwrap_or_default(),
    };
    // The run whose message this turn works for notified (`automations.md`
    // § Runs): it ends Notified, and stays unread until the user opens the
    // conversation.
    if let Some((mark, _)) = shard.automation_input(&conversation)
        && shard.services().control.notify_run(mark.run_id).await.map_err(failed)?
    {
        shard.mark(demi_backend_page_sync::Part::Automations);
    }
    // Counted now, so the agent is told at once; delivery goes on in the
    // background, and a browser its push service no longer knows is
    // dropped then.
    let recipients = shard.recipients(&message).await.map_err(failed)?;
    match recipients.len() {
        0 => {
            let what = "no browser of the user takes the agent's notifications now; `demi notify status` says why";
            port.stderr(warning(&call.invocation.path.join(" "), what)).await?;
            return Ok(0);
        }
        1 => port.stdout("Sending to 1 browser.\n").await?,
        n => port.stdout(format!("Sending to {n} browsers.\n")).await?,
    }
    shard.push(message, Some(recipients));
    Ok(0)
}

async fn status(shard: Rc<Shard>, call: Call<NoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let subscriptions = shard
        .services()
        .push
        .subscriptions(shard.user().clone())
        .await
        .map_err(failed)?;
    let agent_notifies = subscriptions
        .iter()
        .filter(|subscription| subscription.events.agent_notifies)
        .count();
    let status = Status {
        reachable: agent_notifies > 0,
        browsers: subscriptions.len(),
        agent_notifies,
    };
    let command = call.invocation.path.join(" ");
    let advice = match (status.browsers, status.agent_notifies) {
        (0, _) => Some("notifications are off in every browser of the user; the user turns them on in Settings › Notifications"),
        (_, 0) => Some("no browser of the user takes the agent's notifications; the user turns on The agent notifies in Settings › Notifications"),
        _ => None,
    };
    if let Some(advice) = advice {
        port.stderr(warning(&command, advice)).await?;
    }
    if call.invocation.json {
        let text = serde_json::to_string(&status).map_err(failed)?;
        port.stdout(format!("{text}\n")).await?;
        return Ok(0);
    }
    let mut fields = Fields::new();
    fields
        .field("Reachable", status.reachable)
        .field("Browsers", status.browsers)
        .field("Agent notifications", status.agent_notifies);
    port.stdout(fields.render()).await?;
    Ok(0)
}
