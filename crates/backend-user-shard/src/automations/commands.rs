//! The product's `demi automation` group (`automations.md` § The agent's
//! commands): the agent lists and shows the user's automations, creates,
//! changes and deletes them with the user's Manage Automations, and runs
//! one now. A run's own agent may list automations and show its own, never
//! create, change or delete one: what started it, such as another system's
//! call, must not make more.

use std::rc::{Rc, Weak};

use demi_agent_tools::ModelChoice;
use demi_backend_host_access::host_commands::conversation_of;
use demi_host_interface::{Call, GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_shared_types::{AutomationId, Timestamp};
use demi_web_api_protocol::automations::{
    AutomationDto, AutomationPatch, AutomationTarget, RunState, Schedule, Trigger, Weekday,
};
use demi_web_api_protocol::conversations::ConversationTarget;
use demi_web_api_protocol::devices::DeviceKind;
use demi_web_api_protocol::ids::ConversationId;
use demi_web_api_protocol::text::Trimmed;
use garde::Validate;
use jiff::civil::DateTime;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::AutomationError;
use crate::conversation::organize::{failed, verb};
use crate::shard::Shard;
use crate::shard::credential_commands::print_json;

/// The permission category of `create`, `edit` and `delete`.
pub(crate) const MANAGE: &str = "automation.manage";

const MANAGE_ACTION: &str = "manage automations";

const MANAGE_DESCRIPTION: &str = "Manage Automations lets the agents of this conversation create, change and delete your automations, which start the agent at a time, when a command's output changes, or when a URL is called, in any of your conversations or devices.";

const SUMMARY: &str = "The user's automations: the agent started at a time, when a command's output changes, or when a URL is called, in a new conversation each run or in an existing one. List, show, create, change, run now and delete them.";

/// The group's entry in the model's capability index.
const ENTRY: &str = "Starts the agent without the user, at a time (once, daily on chosen weekdays, every interval, or cron), when a command's output changes, or when another system calls a URL, in a new conversation each run or in this one. Use it when the user asks for something to happen on a schedule or on an event: restate the schedule in words and the target, and create it once the user agrees.";

const LIST_SUMMARY: &str = "List the user's automations: id, name, trigger, target, whether it is on, and its last run.";
const SHOW_SUMMARY: &str = "Show one automation: its trigger, target, model, prompt, state and latest runs. A run's own agent may show only its own automation.";
const CREATE_SUMMARY: &str = "Create an automation, on at once, with the prompt each run sends on stdin. One trigger: --at, --daily [--days], --every, --cron, --watch with --every for a change, or --url. Without --conversation or --new it wakes this conversation. Restate the schedule in words and get the user's agreement first. Needs the user's permission.";
const EDIT_SUMMARY: &str = "Change the parts the options name and keep the others; a trigger option replaces the trigger, stdin replaces the prompt. --paused pauses it, --on turns it on. Needs the user's permission.";
const RUN_SUMMARY: &str = "Run an automation now, as the user's Run Now does, whatever its schedule, its limit or whether it is paused.";
const DELETE_SUMMARY: &str = "Delete an automation and its runs' records; their conversations stay and join the sidebar. Only when the user asks for it. Needs the user's permission.";

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NoArgs {}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct IdArgs {
    id: String,
}

/// The options `create` and `edit` share.
#[derive(Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct PartsArgs {
    name: Option<String>,
    at: Option<String>,
    daily: Option<String>,
    days: Option<String>,
    every: Option<String>,
    cron: Option<String>,
    tz: Option<String>,
    watch: Option<String>,
    url: Option<bool>,
    max_runs_per_day: Option<u32>,
    no_limit: Option<bool>,
    conversation: Option<String>,
    new: Option<bool>,
    device: Option<String>,
    directory: Option<String>,
    model: Option<String>,
    effort: Option<String>,
    prompt: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct EditArgs {
    id: String,
    #[serde(flatten)]
    parts: PartsArgs,
    paused: Option<bool>,
    on: Option<bool>,
}

#[derive(Serialize, JsonSchema)]
struct Listing {
    automations: Vec<AutomationDto>,
}

#[derive(Serialize, JsonSchema)]
struct Shown {
    automation: AutomationDto,
    /// A URL trigger's secret, printed once by `create` and by a change
    /// that made it.
    #[serde(skip_serializing_if = "Option::is_none")]
    secret: Option<String>,
    /// A URL trigger's address.
    #[serde(skip_serializing_if = "Option::is_none")]
    address: Option<String>,
}

/// The `automation` group, whose handlers act in `shard`.
pub(crate) fn automation_group(shard: Weak<Shard>) -> GroupBuilder {
    let parts = |leaf: LeafBuilder| {
        leaf.describe("name", "A line the user reads in the list, such as \"Failed payments every weekday\".")
            .describe("at", "Once, at a local time in --tz, such as \"2026-10-18 08:00\".")
            .describe("daily", "Every day at a clock time, such as 09:00; --days narrows the days.")
            .describe("days", "The days of --daily: mon-fri, sat,sun, or mon,wed,fri.")
            .describe("every", "An interval of 1 minute or more, such as 10m, 2h or 1d; with --watch, how often the command runs.")
            .describe("cron", "Five cron fields, such as \"0 9 * * 1-5\", for what the others cannot say.")
            .describe("tz", "An IANA time zone such as Asia/Shanghai; the user's own by default.")
            .describe("watch", "A command whose output change triggers a run, run every --every on the target's device in its directory, as your shell runs it. Its first run only records the output; one over 60 seconds fails.")
            .describe("url", "A URL trigger: another system calls its address with its secret, which create prints once.")
            .describe("max-runs-per-day", "With --watch or --url: at most this many runs a day, counted from midnight in --tz.")
            .describe("no-limit", "Remove the limit on runs a day.")
            .describe("conversation", "Wake this existing conversation, by its id, with each run's message.")
            .describe("new", "A new conversation each run; --device and --directory say where, the Cloud's own directory by default.")
            .describe("device", "With --new: the device, by its name, or Cloud.")
            .describe("directory", "With --new: an absolute directory on the device, such as a project's.")
            .describe("model", "With --new: a model id `demi providers list` lists, which each run's new conversation starts with.")
            .describe("effort", "With --model: an effort the model lists, or off.")
            .describe("prompt", "The message each run sends, written as the user would write it; stdin.")
            .stdin_field("prompt")
    };
    GroupBuilder::new("automation", SUMMARY)
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
                .input::<IdArgs>()
                .describe("id", "The automation's id, as `list` prints it.")
                .positionals(["id"])
                .json_output::<Shown>()
                .bind(TypedRpc::new(verb(shard.clone(), show))),
        )
        .leaf(
            parts(LeafBuilder::rpc("create", CREATE_SUMMARY).input::<PartsArgs>())
                .permission(MANAGE)
                .json_output::<Shown>()
                .bind(TypedRpc::new(verb(shard.clone(), create))),
        )
        .leaf(
            parts(LeafBuilder::rpc("edit", EDIT_SUMMARY).input::<EditArgs>())
                .describe("id", "The automation's id.")
                .describe("paused", "Pause it: it runs only when run now.")
                .describe("on", "Turn it on.")
                .positionals(["id"])
                .permission(MANAGE)
                .json_output::<Shown>()
                .bind(TypedRpc::new(verb(shard.clone(), edit))),
        )
        .leaf(
            LeafBuilder::rpc("run", RUN_SUMMARY)
                .input::<IdArgs>()
                .describe("id", "The automation's id.")
                .positionals(["id"])
                .bind(TypedRpc::new(verb(shard.clone(), run))),
        )
        .leaf(
            LeafBuilder::rpc("delete", DELETE_SUMMARY)
                .input::<IdArgs>()
                .describe("id", "The automation's id.")
                .positionals(["id"])
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard, delete))),
        )
}

/// Refuses a change by a run's own agent: one whose conversation works for
/// an automation's message, until the user next writes to it
/// (`automations.md` § What a run sends).
async fn not_a_run(shard: &Shard, call_conversation: &ConversationId, verb: &str) -> Result<(), RpcError> {
    if shard.automation_input(call_conversation).is_none() {
        Ok(())
    } else {
        Err(failed(format!(
            "an automation's run may not {verb} automations; ask the user in another conversation"
        )))
    }
}

fn refused(error: AutomationError) -> RpcError {
    failed(error)
}

fn id_of(text: &str) -> Result<AutomationId, RpcError> {
    AutomationId::try_from(text).map_err(|_| failed("give the automation's id, as `list` prints it"))
}

/// A trigger in words, as `list` and `show` print it.
fn trigger_words(automation: &AutomationDto) -> String {
    let zone = &automation.time_zone;
    match &automation.trigger {
        Trigger::Time { schedule } => match schedule {
            Schedule::Once { at } => format!("once at {at} ({zone})"),
            Schedule::Daily { time, days } if days.len() == 7 => format!("every day at {time} ({zone})"),
            Schedule::Daily { time, days } => format!(
                "at {time} on {} ({zone})",
                days.iter().map(|day| day.as_str()).collect::<Vec<_>>().join(",")
            ),
            Schedule::Every { minutes } => format!("every {minutes} min"),
            Schedule::Cron { expression } => format!("cron \"{expression}\" ({zone})"),
        },
        Trigger::Change { command, every_minutes } => {
            format!("every {every_minutes} min when the output of `{command}` changes")
        }
        Trigger::Url {} => "when its URL is called".into(),
    }
}

fn target_words(target: &AutomationTarget) -> String {
    match target {
        AutomationTarget::Existing { conversation_id } => format!("wakes conversation {conversation_id}"),
        AutomationTarget::New { target } => match target {
            ConversationTarget::Cloud { path: None } => "a new conversation on the Cloud".into(),
            ConversationTarget::Cloud { path: Some(path) } => format!("a new conversation on the Cloud in {path}"),
            ConversationTarget::Device { device_id, path } => {
                format!("a new conversation on device {device_id} in {path}")
            }
            ConversationTarget::Workspace { workspace_id } => {
                format!("a new conversation in project {workspace_id}")
            }
        },
    }
}

fn state_words(state: RunState) -> &'static str {
    match state {
        RunState::Running => "running",
        RunState::Done => "done",
        RunState::Notified => "notified",
        RunState::Skipped => "skipped",
        RunState::Failed => "failed",
    }
}

fn line(automation: &AutomationDto) -> String {
    let state = if automation.paused { "paused" } else { "on" };
    let last = automation
        .runs
        .first()
        .map(|run| format!("; last run {} at {}", state_words(run.state), run.at))
        .unwrap_or_default();
    format!(
        "{}  {}\n  {}; {}; {state}{last}\n",
        automation.id,
        automation.name,
        trigger_words(automation),
        target_words(&automation.target)
    )
}

fn address(shard: &Shard, automation: &AutomationDto) -> Option<String> {
    if !matches!(automation.trigger, Trigger::Url {}) {
        return None;
    }
    let base = shard.services().public_url.get()?;
    Some(format!("{}/hooks/{}", base.as_str().trim_end_matches('/'), automation.id))
}

async fn list(shard: Rc<Shard>, call: Call<NoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let automations = shard.automation_list().await.map_err(failed)?;
    if call.invocation.json {
        return print_json(&port, &Listing { automations }).await;
    }
    if automations.is_empty() {
        port.stdout("The user has no automations.\n").await?;
        return Ok(0);
    }
    let text: String = automations.iter().map(line).collect();
    port.stdout(text).await?;
    Ok(0)
}

async fn print_shown(
    shard: &Shard,
    port: &RpcPort,
    json: bool,
    automation: AutomationDto,
    secret: Option<String>,
) -> Result<u8, RpcError> {
    let address = address(shard, &automation);
    if json {
        return print_json(port, &Shown { automation, secret, address }).await;
    }
    let mut text = line(&automation);
    if let Some(address) = &address {
        text.push_str(&format!("  address: {address}\n"));
    }
    if let Some(secret) = &secret {
        text.push_str(&format!(
            "  secret: {secret}\n  (printed once; the user can show it again on the automation's page)\n"
        ));
    }
    if let Some(limit) = automation.max_runs_per_day {
        text.push_str(&format!("  at most {limit} runs a day\n"));
    }
    if let Some(next) = automation.next_at {
        text.push_str(&format!("  next: {next}\n"));
    }
    if let Some(model) = &automation.model {
        text.push_str(&format!("  model: {} ({})\n", model.model_id, model.provider_id));
    }
    text.push_str(&format!("  prompt:\n{}\n", indent(&automation.prompt)));
    for run in automation.runs.iter().take(10) {
        text.push_str(&format!(
            "  run {} {}{}\n",
            run.at,
            state_words(run.state),
            run.line.as_deref().map(|line| format!(": {line}")).unwrap_or_default()
        ));
    }
    port.stdout(text).await?;
    Ok(0)
}

fn indent(text: &str) -> String {
    text.lines().map(|line| format!("    {line}")).collect::<Vec<_>>().join("\n")
}

async fn show(shard: Rc<Shard>, call: Call<IdArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let id = id_of(&call.args.id)?;
    let conversation = conversation_of(&call.invocation)?;
    if let Some((mark, _)) = shard.automation_input(&conversation)
        && mark.automation_id != id
    {
        return Err(failed("an automation's run may show only its own automation"));
    }
    let automation = shard
        .automation_list()
        .await
        .map_err(failed)?
        .into_iter()
        .find(|automation| automation.id == id)
        .ok_or_else(|| failed(format!("{id}: the user has no automation of that id")))?;
    print_shown(&shard, &port, call.invocation.json, automation, None).await
}

/// The local time `text` names in `zone`: `2026-10-18 08:00`, or an
/// instant with its offset.
fn moment(text: &str, zone: &str) -> Result<Timestamp, RpcError> {
    if let Ok(at) = text.parse::<jiff::Timestamp>() {
        return Ok(Timestamp::truncate(at));
    }
    let local: DateTime = text
        .replacen(' ', "T", 1)
        .parse()
        .map_err(|_| failed(format!("--at {text:?}: give a local time such as \"2026-10-18 08:00\"")))?;
    let zone = super::schedule::zone(zone).map_err(failed)?;
    let zoned = local.to_zoned(zone).map_err(failed)?;
    Ok(Timestamp::truncate(zoned.timestamp()))
}

/// `10m`, `2h`, `1d` or a number of minutes, in minutes.
fn minutes(text: &str) -> Result<u32, RpcError> {
    let usage = || failed(format!("--every {text:?}: give an interval such as 10m, 2h or 1d"));
    let text = text.trim();
    let (number, unit) = match text.find(|character: char| !character.is_ascii_digit()) {
        Some(index) => text.split_at(index),
        None => (text, "m"),
    };
    let number: u32 = number.parse().map_err(|_| usage())?;
    let factor = match unit.trim() {
        "m" | "min" | "minute" | "minutes" => 1,
        "h" | "hour" | "hours" => 60,
        "d" | "day" | "days" => 24 * 60,
        _ => return Err(usage()),
    };
    number.checked_mul(factor).filter(|minutes| *minutes >= 1).ok_or_else(usage)
}

/// `mon-fri`, `sat,sun` or `mon,wed,fri`.
fn days(text: &str) -> Result<Vec<Weekday>, RpcError> {
    let usage = || failed(format!("--days {text:?}: give days such as mon-fri or sat,sun"));
    let mut days = Vec::new();
    for part in text.split(',') {
        let part = part.trim().to_ascii_lowercase();
        match part.split_once('-') {
            Some((from, to)) => {
                let index = |day: &str| Weekday::ALL.iter().position(|each| each.as_str() == day);
                let (from, to) = (index(from).ok_or_else(usage)?, index(to).ok_or_else(usage)?);
                if from > to {
                    return Err(usage());
                }
                days.extend(&Weekday::ALL[from..=to]);
            }
            None => days.push(Weekday::named(&part).ok_or_else(usage)?),
        }
    }
    days.sort();
    days.dedup();
    Ok(days)
}

/// What the options change, as a page's patch would.
async fn patch(
    shard: &Shard,
    conversation: &ConversationId,
    args: PartsArgs,
    zone: String,
    creating: bool,
) -> Result<AutomationPatch, RpcError> {
    let zone = args.tz.clone().unwrap_or(zone);
    let mut chosen = Vec::new();
    if let Some(at) = &args.at {
        chosen.push(Trigger::Time {
            schedule: Schedule::Once { at: moment(at, &zone)? },
        });
    }
    if let Some(time) = &args.daily {
        chosen.push(Trigger::Time {
            schedule: Schedule::Daily {
                time: time.trim().to_owned(),
                days: match &args.days {
                    Some(text) => days(text)?,
                    None => Weekday::ALL.to_vec(),
                },
            },
        });
    } else if args.days.is_some() {
        return Err(failed("--days goes with --daily"));
    }
    if let Some(expression) = &args.cron {
        chosen.push(Trigger::Time {
            schedule: Schedule::Cron {
                expression: expression.trim().to_owned(),
            },
        });
    }
    match (&args.watch, &args.every) {
        (Some(command), Some(every)) => chosen.push(Trigger::Change {
            command: command.clone(),
            every_minutes: minutes(every)?,
        }),
        (Some(_), None) => return Err(failed("--watch needs --every, how often its command runs")),
        (None, Some(every)) => chosen.push(Trigger::Time {
            schedule: Schedule::Every { minutes: minutes(every)? },
        }),
        (None, None) => {}
    }
    if args.url == Some(true) {
        chosen.push(Trigger::Url {});
    }
    if chosen.len() > 1 {
        return Err(failed("give one trigger: --at, --daily, --every, --cron, --watch with --every, or --url"));
    }
    if creating && chosen.is_empty() {
        return Err(failed("give a trigger: --at, --daily, --every, --cron, --watch with --every, or --url"));
    }
    let target = match (&args.conversation, args.new == Some(true)) {
        (Some(_), true) => return Err(failed("--conversation and --new contradict each other; give one")),
        (Some(id), false) => Some(AutomationTarget::Existing {
            conversation_id: ConversationId::try_from(id.as_str())
                .map_err(|_| failed("--conversation: give a conversation's id"))?,
        }),
        (None, true) => Some(AutomationTarget::New {
            target: new_target(shard, args.device.as_deref(), args.directory.clone()).await?,
        }),
        (None, false) if args.device.is_some() || args.directory.is_some() => {
            return Err(failed("--device and --directory go with --new"));
        }
        (None, false) if creating => Some(AutomationTarget::Existing {
            conversation_id: conversation.clone(),
        }),
        (None, false) => None,
    };
    let model = match &args.model {
        Some(model) => Some(Some(
            shard
                .chosen_model(
                    ModelChoice {
                        model: Some(model.clone()),
                        provider: None,
                        effort: args.effort.clone(),
                        tier: None,
                    },
                    None,
                )
                .await
                .map_err(failed)?,
        )),
        None if args.effort.is_some() => return Err(failed("--effort goes with --model")),
        None => None,
    };
    let max_runs_per_day = match (args.max_runs_per_day, args.no_limit == Some(true)) {
        (Some(_), true) => return Err(failed("--max-runs-per-day and --no-limit contradict each other")),
        (Some(limit), false) => Some(Some(limit)),
        (None, true) => Some(None),
        (None, false) => None,
    };
    let prompt = args.prompt.map(|prompt| prompt.trim().to_owned()).filter(|prompt| !prompt.is_empty());
    let patch = AutomationPatch {
        name: args.name.map(Trimmed::from),
        trigger: chosen.pop(),
        time_zone: args.tz.or(creating.then_some(zone)),
        max_runs_per_day,
        target,
        prompt,
        model,
        paused: None,
        stripe_secret: None,
    };
    patch
        .validate()
        .map_err(|report| failed(demi_web_api_protocol::error::problems(&report)))?;
    Ok(patch)
}

/// A new conversation's target on the device `device` names, by its name
/// or Cloud, in `directory`.
async fn new_target(shard: &Shard, device: Option<&str>, directory: Option<String>) -> Result<ConversationTarget, RpcError> {
    let cloud = device.is_none_or(|name| name.eq_ignore_ascii_case("cloud"));
    if cloud {
        return Ok(ConversationTarget::Cloud { path: directory });
    }
    let name = device.expect("a device is named");
    let devices = shard.services().control.paired_devices(shard.user().clone()).await.map_err(failed)?;
    let found = devices
        .iter()
        .find(|record| record.kind == DeviceKind::User && record.name.eq_ignore_ascii_case(name))
        .ok_or_else(|| failed(format!("--device {name:?}: the user has no device of that name")))?;
    let path = match directory {
        Some(path) => path,
        None => shard
            .devices()
            .home(&found.id)
            .ok_or_else(|| failed(format!("--device {name:?}: its home is not known yet; give --directory")))?,
    };
    Ok(ConversationTarget::Device {
        device_id: found.id.clone(),
        path,
    })
}

async fn user_zone(shard: &Shard) -> Result<String, RpcError> {
    let reported = demi_backend_runners::command_context::reported(&shard.services().control, shard.user())
        .await
        .map_err(failed)?;
    Ok(reported.locale.time_zone)
}

async fn create(shard: Rc<Shard>, call: Call<PartsArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    not_a_run(&shard, &conversation, "create").await?;
    let zone = user_zone(&shard).await?;
    let mut patch = patch(&shard, &conversation, call.args, zone, true).await?;
    // The user's grant is the confirmation: it is on at once.
    patch.paused = Some(false);
    let (automation, secret) = shard.create_automation(patch).await.map_err(refused)?;
    print_shown(&shard, &port, call.invocation.json, automation, secret).await
}

async fn edit(shard: Rc<Shard>, call: Call<EditArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    not_a_run(&shard, &conversation, "change").await?;
    let id = id_of(&call.args.id)?;
    let current = shard.stored_automation(&id).await.map_err(refused)?;
    let mut patch = patch(&shard, &conversation, call.args.parts, current.time_zone, false).await?;
    patch.paused = match (call.args.paused == Some(true), call.args.on == Some(true)) {
        (true, true) => return Err(failed("--paused and --on contradict each other; give one")),
        (true, false) => Some(true),
        (false, true) => Some(false),
        (false, false) => None,
    };
    if patch == AutomationPatch::default() {
        return Err(failed("nothing to change: give an option, or the prompt on stdin"));
    }
    let (automation, secret) = shard.change_automation(&id, patch).await.map_err(refused)?;
    print_shown(&shard, &port, call.invocation.json, automation, secret).await
}

async fn run(shard: Rc<Shard>, call: Call<IdArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let id = id_of(&call.args.id)?;
    let conversation = conversation_of(&call.invocation)?;
    not_a_run(&shard, &conversation, "run").await?;
    let automation = shard.run_automation_now(&id).await.map_err(refused)?;
    port.stdout(format!("Started a run of \u{201c}{}\u{201d}.\n", automation.name)).await?;
    Ok(0)
}

async fn delete(shard: Rc<Shard>, call: Call<IdArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    not_a_run(&shard, &conversation, "delete").await?;
    let id = id_of(&call.args.id)?;
    let automation = shard.stored_automation(&id).await.map_err(refused)?;
    shard.delete_automation(&id).await.map_err(refused)?;
    port.stdout(format!("Deleted the automation \u{201c}{}\u{201d}.\n", automation.name)).await?;
    Ok(0)
}
