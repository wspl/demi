//! The product's `demi notify` group.
//!
//! - `send` sends the user a notification from the conversation.
//! - `status` says whether one would reach the user.
//!
//! Neither needs a permission, and no command changes which events notify, which
//! each browser chooses. When either finds that no browser takes the agent's
//! notifications, the conversation shows the user a card that turns them on,
//! unless the user dismissed it there.


use std::rc::{Rc, Weak};

use demi_backend_host_access::host_commands::conversation_of;
use demi_host_interface::{Call, GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_shared_types::output::{Fields, warning};
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::push::PushMessage;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::Shard;
use crate::conversation::organize::{failed, refused, verb};

/// The longest title, and the longest body, in characters.
///
/// Bounded because the push services take a message of about 4 KB at most.
const TITLE_MAX: usize = 80;
const BODY_MAX: usize = 200;

const SUMMARY: &str = "Notifications to the user: send one from this conversation, which reaches the user's browsers and phone even with Demi closed, or check whether one would reach the user.";

/// The group's entry in the model's capability index.
const ENTRY: &str = "A notification to the user's browsers and phone.";

const SEND_SUMMARY: &str = "Send the user a notification: its title and body show in the system's notifications, and a click opens this conversation. At most 3 a minute and 30 an hour per conversation. When no browser takes it, the conversation shows the user a card that turns notifications on.";

const STATUS_SUMMARY: &str = "Say whether a notification would reach the user: how many of the user's browsers have notifications on, and how many of them take the agent's notifications, with a warning on stderr when none does; the conversation then shows the user a card that turns them on. The user chooses this in Settings › Notifications; no command changes it.";

/// What the warnings add: the user is asked already.
const CARD: &str = "the conversation shows the user a card that turns them on, unless the user dismissed it here";

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
        return Err(refused("the title is blank"));
    }
    let message = PushMessage::Agent {
        conversation_id: conversation.clone(),
        id: uuid::Uuid::new_v4().to_string(),
        title,
        body: body.map(|body| body.trim().to_owned()).unwrap_or_default(),
    };
    // The run whose message this turn works for notified: it ends Notified.
    if let Some((mark, _)) = shard.schedule_input(&conversation)
        && shard.services().control.notify_run(mark.run_id).await.map_err(failed)?
    {
        shard.mark(demi_backend_page_sync::Part::Metadata(conversation.clone()));
    }
    // Counted now, so the agent is told at once; delivery goes on in the
    // background, and a browser its push service no longer knows is
    // dropped then.
    let in_use = shard.notifications().in_use();
    let recipients = shard.recipients(&message, &in_use).await.map_err(failed)?;
    match (recipients.now.len(), recipients.shown) {
        (0, 0) => {
            ask_to_turn_on(&shard, &conversation).await?;
            let what = format!("no browser of the user takes the agent's notifications now; {CARD}");
            port.stderr(warning(&call.invocation.path.join(" "), &what)).await?;
            return Ok(0);
        }
        // The user sees the conversation, on every browser that would
        // notify, so none does.
        (0, _) => {
            port.stdout("Not sent: the user has this conversation open.\n").await?;
            return Ok(0);
        }
        (1, _) => port.stdout("Sending to 1 browser.\n").await?,
        (n, _) => port.stdout(format!("Sending to {n} browsers.\n")).await?,
    }
    shard.push(message, Some(recipients));
    Ok(0)
}

async fn status(shard: Rc<Shard>, call: Call<NoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
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
        (0, _) => Some("notifications are off in every browser of the user"),
        (_, 0) => Some("no browser of the user takes the agent's notifications"),
        _ => None,
    };
    if let Some(advice) = advice {
        ask_to_turn_on(&shard, &conversation).await?;
        port.stderr(warning(&command, &format!("{advice}; {CARD}"))).await?;
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

/// Shows the card that asks the user to turn notifications on in
/// `conversation`, unless it shows it or the user dismissed it there.
async fn ask_to_turn_on(shard: &Shard, conversation: &ConversationId) -> Result<(), RpcError> {
    if shard
        .services()
        .control
        .raise_notifications_card(conversation.clone())
        .await
        .map_err(failed)?
    {
        shard.mark(demi_backend_page_sync::Part::Metadata(conversation.clone()));
    }
    Ok(())
}
