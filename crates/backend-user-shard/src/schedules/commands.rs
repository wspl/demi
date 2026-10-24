//! The product's `demi schedule` group.
//!
//! The agent lists and shows the schedules of the conversation it works in,
//! creates, changes and deletes them with the user's Manage Schedules, and runs
//! one now. It acts on this conversation's schedules only, and its permission is
//! the one guard: a schedule it makes wakes the agent, the one that made it
//! included.

use std::rc::{Rc, Weak};

use demi_backend_host_access::host_commands::conversation_of;
use demi_host_interface::{Call, GroupBuilder, LeafBuilder, RpcError, RpcPort, TypedRpc};
use demi_shared_types::{
    ScheduleId, Timestamp,
    output::{Cell, Fields, Table, warning},
};
use demi_web_api_protocol::schedules::{
    FirstOrLast, HolidayPlace, IfBusy, MonthDay, Repeat, RepeatUnit, RunState, ScheduleDto,
    SchedulePatch, Task, Timing, Trigger, WeekOfMonth, Weekday,
};
use demi_web_api_protocol::text::Trimmed;
use garde::Validate;
use jiff::civil::DateTime;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use demi_backend_database::schedules::StoredSchedule;

use super::ScheduleError;
use crate::conversation::organize::{failed, verb};
use crate::shard::Shard;
use crate::shard::credential_commands::print_json;

/// The permission category of `create`, `edit` and `delete`.
pub(crate) const MANAGE: &str = "schedule.manage";

const MANAGE_ACTION: &str = "manage schedules";

const MANAGE_DESCRIPTION: &str = "Manage Schedules lets the agents of this conversation create, change and delete the schedules that wake it, at a time, when a command's output changes, or when a URL is called.";

const SUMMARY: &str = "This conversation's schedules, which wake it: once, on a repeating cycle with tasks in it, every interval, by cron, when a command's output changes, or when a URL is called. List, show, create, change, run now and delete them.";

/// The group's entry in the model's capability index.
const ENTRY: &str = "Wakes this conversation once, on a repeating cycle, every interval, by cron, when a command's output changes, or on a URL call.";

const LIST_SUMMARY: &str = "List this conversation's schedules: id, whether it is on, its trigger's kind, its next and last run, and its name; show gives the timing and the tasks.";
const SHOW_SUMMARY: &str = "Show one schedule: its trigger, a repeat's tasks with their ids, its prompt, state and latest runs; a URL trigger's address.";
const CREATE_SUMMARY: &str = "Create a schedule, on at once, with the message each run sends on stdin. One trigger: --repeat with --task-at for its first task, --at, --every, --cron, --watch with --every for a change, or --url. Needs the user's permission.";
const EDIT_SUMMARY: &str = "Change the parts the options name and keep the others. --repeat changes a repeat's cycle and keeps its tasks; another trigger option replaces the trigger. A repeat's task: --task picks one by its id (the only one by default) for --task-at, --holidays and stdin; --add-task adds one; --delete-task removes one. Without tasks, stdin replaces the message. --paused pauses it, --on turns it on. Needs the user's permission.";
const RUN_SUMMARY: &str = "Run a schedule now, whatever its timing, its limit or whether it is paused; --task picks a repeat's task when it has several.";
const DELETE_SUMMARY: &str = "Delete a schedule and its runs' records. Needs the user's permission.";

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
    repeat: Option<String>,
    start: Option<String>,
    task_at: Option<String>,
    holidays: Option<String>,
    at: Option<String>,
    every: Option<String>,
    cron: Option<String>,
    tz: Option<String>,
    watch: Option<String>,
    url: Option<bool>,
    max_runs_per_day: Option<u32>,
    no_limit: Option<bool>,
    if_busy: Option<String>,
    prompt: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct EditArgs {
    id: String,
    #[serde(flatten)]
    parts: PartsArgs,
    task: Option<String>,
    add_task: Option<bool>,
    delete_task: Option<String>,
    paused: Option<bool>,
    on: Option<bool>,
    new_secret: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, rename_all = "kebab-case")]
struct RunArgs {
    id: String,
    task: Option<String>,
}

/// `show --json`, and the answer of `create` and `edit`: the schedule
/// with its URL trigger's address and secret.
#[derive(Serialize, JsonSchema)]
struct Shown {
    #[serde(flatten)]
    schedule: ScheduleDto,
    /// A URL trigger's secret.
    #[serde(skip_serializing_if = "Option::is_none")]
    secret: Option<String>,
    /// A URL trigger's address.
    #[serde(skip_serializing_if = "Option::is_none")]
    address: Option<String>,
}

/// The `schedule` group, whose handlers act in `shard`.
pub(crate) fn schedule_group(shard: Weak<Shard>) -> GroupBuilder {
    let parts = |leaf: LeafBuilder| {
        leaf.describe("name", "A line the user reads in the list, such as \"Failed payments every weekday\".")
            .describe("repeat", "A repeating cycle: day, week, month or year, or a count of them such as \"2 weeks\" or \"3 months\". Its tasks run in each cycle's first unit.")
            .describe("start", "With a cycle of several units: the day of its first cycle, such as 2026-01-05; today by default.")
            .describe("task-at", "A task's moment in the cycle, its time last: \"09:00\" (days); \"mon-fri 09:00\" or \"mon,wed 10:00\" (weeks); \"15 09:00\", \"last 17:00\", \"second-tue 10:00\", \"last-fri 10:00\", \"first-working 09:00\" or \"last-working 17:00\" (months); the month first for years, \"mar 15 09:00\" or \"dec last-working 17:00\". A date a month lacks is skipped that month.")
            .describe("holidays", "A task's working days follow a country's working week and skip its public holidays: a country code such as CN, or a country and a region such as DE/BY; none for Monday to Friday.")
            .describe("at", "Once, at a local time in --tz, such as \"2026-10-18 08:00\".")
            .describe("every", "An interval of 1 minute or more, such as 10m, 2h or 1d; with --watch, how often the command runs.")
            .describe("cron", "Five cron fields, such as \"0 9 * * 1-5\", for what the others cannot say.")
            .describe("tz", "An IANA time zone such as Asia/Shanghai; the user's own by default.")
            .describe("watch", "A command whose output change triggers a run, run every --every on this conversation's Host in its directory, as your shell runs it. Its first run only records the output; one over 60 seconds fails.")
            .describe("url", "A URL trigger: another system calls its address with its secret, which create prints once; edit --new-secret makes a new one.")
            .describe("max-runs-per-day", "With --watch or --url: at most this many runs a day, counted from midnight in --tz.")
            .describe("no-limit", "Remove the limit on runs a day.")
            .describe("if-busy", "While the conversation works: queue, the default, waits for its running turn to end; steer reaches the running turn, as the composer's Steer does.")
            .describe("prompt", "The message each run sends, a repeat's task's or a single trigger's, written as the user would write it; stdin.")
            .stdin_field("prompt")
    };
    GroupBuilder::new("schedule", SUMMARY)
        .index_entry(ENTRY)
        .permission(MANAGE, MANAGE_ACTION, MANAGE_DESCRIPTION)
        .leaf(
            LeafBuilder::rpc("list", LIST_SUMMARY)
                .input::<NoArgs>()
                .json_output::<Vec<ScheduleDto>>()
                .bind(TypedRpc::new(verb(shard.clone(), list))),
        )
        .leaf(
            LeafBuilder::rpc("show", SHOW_SUMMARY)
                .input::<IdArgs>()
                .describe("id", "The schedule's id, as `list` prints it.")
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
                .describe("id", "The schedule's id.")
                .describe("task", "The repeat's task, by its id as show prints it, that --task-at, --holidays and stdin change.")
                .describe("add-task", "Add a task to the repeat at --task-at, Monday 09:00 by default, with stdin as its message.")
                .describe("delete-task", "Remove the repeat's task of this id; the last one stays.")
                .describe("paused", "Pause it: it runs only when run now.")
                .describe("on", "Turn it on.")
                .describe("new-secret", "Make a new secret for a URL trigger and print it once; the old one stops working at once.")
                .positionals(["id"])
                .permission(MANAGE)
                .json_output::<Shown>()
                .bind(TypedRpc::new(verb(shard.clone(), edit))),
        )
        .leaf(
            LeafBuilder::rpc("run", RUN_SUMMARY)
                .input::<RunArgs>()
                .describe("id", "The schedule's id.")
                .describe("task", "The repeat's task to run, by its id as show prints it.")
                .positionals(["id"])
                .bind(TypedRpc::new(verb(shard.clone(), run))),
        )
        .leaf(
            LeafBuilder::rpc("delete", DELETE_SUMMARY)
                .input::<IdArgs>()
                .describe("id", "The schedule's id.")
                .positionals(["id"])
                .permission(MANAGE)
                .bind(TypedRpc::new(verb(shard, delete))),
        )
}

fn refused(error: ScheduleError) -> RpcError {
    failed(error)
}

fn id_of(text: &str) -> Result<ScheduleId, RpcError> {
    ScheduleId::try_from(text).map_err(|_| failed("give the schedule's id, as `list` prints it"))
}

/// A trigger's kind, as `list` and `show` print it: `once`, `repeat`,
/// `every`, `cron`, `watch` or `url`.
fn trigger_kind(trigger: &Trigger) -> &'static str {
    match trigger {
        Trigger::Time { timing } => match timing {
            Timing::Once { .. } => "once",
            Timing::Repeat { .. } => "repeat",
            Timing::Every { .. } => "every",
            Timing::Cron { .. } => "cron",
        },
        Trigger::Change { .. } => "watch",
        Trigger::Url {} => "url",
    }
}

/// A schedule's status: `on` or `paused`.
fn status(schedule: &ScheduleDto) -> &'static str {
    if schedule.paused { "paused" } else { "on" }
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

fn address(shard: &Shard, schedule: &ScheduleDto) -> Option<String> {
    if !matches!(schedule.trigger, Trigger::Url {}) {
        return None;
    }
    let base = shard.services().public_url.get()?;
    Some(format!("{}/hooks/{}", base.as_str().trim_end_matches('/'), schedule.id))
}

async fn list(shard: Rc<Shard>, call: Call<NoArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    let schedules = shard.schedule_list(&conversation).await.map_err(failed)?;
    if call.invocation.json {
        return print_json(&port, &schedules).await;
    }
    let mut table = Table::new(["id", "status", "trigger", "next run", "last run", "name"]);
    for schedule in &schedules {
        table.row::<Cell>([
            schedule.id.to_string().into(),
            status(schedule).into(),
            trigger_kind(&schedule.trigger).into(),
            schedule.next_at.into(),
            schedule.runs.first().map(|run| run.at).into(),
            schedule.name.as_str().into(),
        ]);
    }
    port.stdout(table.render()).await?;
    Ok(0)
}

async fn print_shown(
    shard: &Shard,
    port: &RpcPort,
    command: &str,
    json: bool,
    schedule: ScheduleDto,
    secret: Option<String>,
) -> Result<u8, RpcError> {
    let address = address(shard, &schedule);
    if secret.is_some() {
        port.stderr(warning(command, "the secret is printed once; edit --new-secret makes a new one"))
            .await?;
    }
    if json {
        return print_json(port, &Shown { schedule, secret, address }).await;
    }
    let mut fields = Fields::new();
    fields
        .field("ID", schedule.id.to_string())
        .field("Name", schedule.name.as_str())
        .field("Status", status(&schedule))
        .field("Trigger", trigger_kind(&schedule.trigger));
    match &schedule.trigger {
        Trigger::Time { timing } => match timing {
            Timing::Once { at } => fields.field("At", *at),
            Timing::Repeat { repeat, .. } => fields
                .field("Repeat", format!("every {} {}", repeat.every, repeat.unit.as_str()))
                .field("First cycle", repeat.start.as_deref()),
            Timing::Every { minutes } => fields.field("Every (min)", *minutes),
            Timing::Cron { expression } => fields.field("Cron", expression.as_str()),
        },
        Trigger::Change { command, every_minutes } => {
            fields.field("Command", command.as_str()).field("Every (min)", *every_minutes)
        }
        Trigger::Url {} => fields
            .field("Address", address.as_deref())
            .field("Secret", secret.as_deref()),
    };
    fields
        .field("Time zone", schedule.time_zone.as_str())
        .field("Max runs per day", schedule.max_runs_per_day);
    fields
        .field("If busy", match schedule.if_busy {
            IfBusy::Queue => "queue",
            IfBusy::Steer => "steer",
        })
        .field("Next run", schedule.next_at)
        .field("Created", schedule.created_at);
    let mut runs = Table::new(["at", "task", "state", "line"]);
    // Ten of the runs, because the table goes into the agent's context window.
    for run in schedule.runs.iter().take(10) {
        runs.row::<Cell>([
            run.at.into(),
            run.task_id.as_deref().into(),
            state_words(run.state).into(),
            run.line.as_deref().into(),
        ]);
    }
    if let Trigger::Time {
        timing: Timing::Repeat { repeat, tasks },
    } = &schedule.trigger
    {
        // Each task by its id, its moment as --task-at takes it, and its message.
        let mut table = Table::new(["id", "task at", "holidays", "message"]);
        for task in tasks {
            table.row::<Cell>([
                task.id.as_str().into(),
                task_at_text(task, repeat.unit).into(),
                task.holidays
                    .as_ref()
                    .map(|place| match &place.region {
                        Some(region) => format!("{}/{region}", place.country),
                        None => place.country.clone(),
                    })
                    .into(),
                task.prompt.as_str().into(),
            ]);
        }
        fields.table("Tasks", &table).table("Runs", &runs);
    } else {
        fields.table("Runs", &runs).text("Prompt", &schedule.prompt);
    }
    port.stdout(fields.render()).await?;
    Ok(0)
}

async fn show(shard: Rc<Shard>, call: Call<IdArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let id = id_of(&call.args.id)?;
    let conversation = conversation_of(&call.invocation)?;
    let schedule = shard
        .schedule_list(&conversation)
        .await
        .map_err(failed)?
        .into_iter()
        .find(|schedule| schedule.id == id)
        .ok_or_else(|| failed(format!("{id}: this conversation has no schedule of that id")))?;
    print_shown(&shard, &port, &call.invocation.path.join(" "), call.invocation.json, schedule, None).await
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
    let zone = super::plan::zone(zone).map_err(failed)?;
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

/// A weekly task's days: `mon-fri`, `sat,sun` or `mon,wed,fri`.
fn days(text: &str) -> Result<Vec<Weekday>, RpcError> {
    let usage = || failed(format!("--task-at {text:?}: give days such as mon-fri, sat,sun or mon,wed,fri"));
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

/// What `edit` does to a repeat's tasks.
#[derive(Default)]
struct TaskOps {
    /// The task --task-at, --holidays and stdin change.
    task: Option<String>,
    add: bool,
    delete: Option<String>,
}

impl TaskOps {
    fn any(&self) -> bool {
        self.task.is_some() || self.add || self.delete.is_some()
    }
}

/// What the options change, as a page's patch would, on `current` when
/// they edit one.
async fn patch(
    shard: &Shard,
    args: PartsArgs,
    zone: String,
    current: Option<&StoredSchedule>,
    ops: TaskOps,
) -> Result<SchedulePatch, RpcError> {
    let creating = current.is_none();
    let zone = args.tz.clone().unwrap_or(zone);
    let mut prompt = args.prompt.map(|prompt| prompt.trim().to_owned()).filter(|prompt| !prompt.is_empty());
    let mut chosen = Vec::new();
    if let Some(at) = &args.at {
        chosen.push(Trigger::Time {
            timing: Timing::Once { at: moment(at, &zone)? },
        });
    }
    if let Some(expression) = &args.cron {
        chosen.push(Trigger::Time {
            timing: Timing::Cron {
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
            timing: Timing::Every { minutes: minutes(every)? },
        }),
        (None, None) => {}
    }
    if args.url == Some(true) {
        chosen.push(Trigger::Url {});
    }
    let cycle = args.repeat.as_deref().map(cycle).transpose()?;
    let triggers = chosen.len() + usize::from(cycle.is_some());
    if triggers > 1 {
        return Err(failed("give one trigger: --repeat, --at, --every, --cron, --watch with --every, or --url"));
    }
    if creating && triggers == 0 {
        return Err(failed("give a trigger: --repeat, --at, --every, --cron, --watch with --every, or --url"));
    }

    // The repeat the change leaves, with its tasks: a new or changed cycle,
    // or the current one while no single trigger replaces it.
    let current_repeat = current.and_then(|schedule| match &schedule.trigger {
        Trigger::Time {
            timing: Timing::Repeat { repeat, tasks },
        } => Some((repeat.clone(), tasks.clone())),
        _ => None,
    });
    let current_prompt = current.map(|schedule| schedule.prompt.clone()).unwrap_or_default();
    let today = || {
        let zone = super::plan::zone(&zone).map_err(failed)?;
        Ok::<_, RpcError>(shard.services().control.now().to_jiff().to_zoned(zone).date().to_string())
    };
    let start = |every: u32, kept: Option<&str>| -> Result<Option<String>, RpcError> {
        match (&args.start, every > 1) {
            (Some(day), true) => {
                let day = day.trim();
                day.parse::<jiff::civil::Date>()
                    .map_err(|_| failed(format!("--start {day:?}: give a day such as 2026-01-05")))?;
                Ok(Some(day.to_owned()))
            }
            (Some(_), false) => Err(failed("--start goes with a cycle of several units, such as --repeat \"2 weeks\"")),
            (None, true) => Ok(Some(match kept {
                Some(kept) => kept.to_owned(),
                None => today()?,
            })),
            (None, false) => Ok(None),
        }
    };
    let mut repeat = match (cycle, &current_repeat) {
        (Some((every, unit)), Some((kept, tasks))) => Some((
            Repeat { every, unit, start: start(every, kept.start.as_deref())? },
            tasks.clone(),
        )),
        // A single trigger's message becomes the repeat's one task's.
        (Some((every, unit)), None) => Some((
            Repeat { every, unit, start: start(every, None)? },
            vec![super::new_task(prompt.take().unwrap_or(current_prompt.clone()))],
        )),
        (None, Some((kept, tasks))) if chosen.is_empty() => {
            let repeat = match &args.start {
                Some(_) => Repeat { start: start(kept.every, kept.start.as_deref())?, ..kept.clone() },
                None => kept.clone(),
            };
            Some((repeat, tasks.clone()))
        }
        _ => None,
    };
    let task_parts = args.task_at.is_some() || args.holidays.is_some();
    let mut trigger = chosen.pop();
    let mut schedule_prompt = None;
    match &mut repeat {
        Some((cycle_now, tasks)) => {
            if let Some(id) = &ops.delete {
                if !tasks.iter().any(|task| task.id == *id) {
                    return Err(failed(format!("--delete-task {id}: the repeat has no task of that id")));
                }
                if tasks.len() == 1 {
                    return Err(failed("--delete-task: a schedule keeps at least one task"));
                }
                tasks.retain(|task| task.id != *id);
            }
            let chosen_task = if ops.add {
                let task = super::new_task(prompt.take().unwrap_or_default());
                let id = task.id.clone();
                tasks.push(task);
                Some(id)
            } else if let Some(id) = &ops.task {
                if !tasks.iter().any(|task| task.id == *id) {
                    return Err(failed(format!("--task {id}: the repeat has no task of that id")));
                }
                Some(id.clone())
            } else if tasks.len() == 1 {
                Some(tasks[0].id.clone())
            } else if task_parts || prompt.is_some() {
                return Err(failed("the repeat has several tasks: give --task with one's id, as show prints it, or --add-task"));
            } else {
                None
            };
            if let Some(id) = chosen_task {
                let task = tasks.iter_mut().find(|task| task.id == id).expect("the task was found");
                if let Some(text) = &args.task_at {
                    task_at(task, text, cycle_now.unit)?;
                }
                if let Some(text) = &args.holidays {
                    task.holidays = holidays(text)?;
                }
                if let Some(text) = prompt.take() {
                    task.prompt = text;
                }
            }
            let changed = cycle.is_some() || args.start.is_some() || task_parts || ops.any() || current_repeat.as_ref()
                .is_some_and(|(_, kept)| kept != tasks);
            if changed {
                trigger = Some(Trigger::Time {
                    timing: Timing::Repeat {
                        repeat: cycle_now.clone(),
                        tasks: tasks.clone(),
                    },
                });
                // A repeat's tasks hold the messages.
                if current.is_none_or(|schedule| !schedule.prompt.is_empty()) {
                    schedule_prompt = Some(String::new());
                }
            }
        }
        None => {
            if task_parts || ops.any() || args.start.is_some() {
                return Err(failed("--task-at, --holidays, --start and the task options go with a repeat (--repeat)"));
            }
            // A repeat replaced by a single trigger keeps its first task's
            // message, unless stdin gives another.
            schedule_prompt = prompt.take().or_else(|| {
                current_repeat
                    .as_ref()
                    .filter(|_| trigger.is_some())
                    .and_then(|(_, tasks)| tasks.first())
                    .map(|task| task.prompt.clone())
            });
        }
    }

    let if_busy = args.if_busy.as_deref().map(if_busy).transpose()?;
    let max_runs_per_day = match (args.max_runs_per_day, args.no_limit == Some(true)) {
        (Some(_), true) => return Err(failed("--max-runs-per-day and --no-limit contradict each other")),
        (Some(limit), false) => Some(Some(limit)),
        (None, true) => Some(None),
        (None, false) => None,
    };
    let patch = SchedulePatch {
        name: args.name.map(Trimmed::from),
        trigger,
        time_zone: args.tz.or(creating.then_some(zone)),
        max_runs_per_day,
        if_busy,
        prompt: schedule_prompt,
        paused: None,
    };
    patch
        .validate()
        .map_err(|report| failed(demi_web_api_protocol::error::problems(&report)))?;
    Ok(patch)
}

/// `week`, `month`, `2 weeks` or `3months`: a repeat's count and unit.
fn cycle(text: &str) -> Result<(u32, RepeatUnit), RpcError> {
    let usage = || failed(format!("--repeat {text:?}: give day, week, month or year, or a count such as \"2 weeks\""));
    let text = text.trim().to_ascii_lowercase();
    let split = text.find(|character: char| !character.is_ascii_digit()).unwrap_or(text.len());
    let (count, unit) = text.split_at(split);
    let every = if count.is_empty() { 1 } else { count.parse().map_err(|_| usage())? };
    let unit = match unit.trim().trim_end_matches('s') {
        "day" => RepeatUnit::Days,
        "week" => RepeatUnit::Weeks,
        "month" => RepeatUnit::Months,
        "year" => RepeatUnit::Years,
        _ => return Err(usage()),
    };
    Ok((every, unit))
}

/// Sets `task`'s moment from `--task-at`, as the repeat's `unit` reads one.
fn task_at(task: &mut Task, text: &str, unit: RepeatUnit) -> Result<(), RpcError> {
    let usage = || {
        failed(format!(
            "--task-at {text:?}: give {}",
            match unit {
                RepeatUnit::Days => "a time such as 09:00",
                RepeatUnit::Weeks => "days and a time such as \"mon-fri 09:00\"",
                RepeatUnit::Months => "a day and a time such as \"15 09:00\", \"last-working 17:00\" or \"second-tue 10:00\"",
                RepeatUnit::Years => "a month, a day and a time such as \"mar 15 09:00\"",
            }
        ))
    };
    let words: Vec<&str> = text.split_whitespace().collect();
    let (time, rest) = words.split_last().ok_or_else(usage)?;
    let valid_time = time.len() == 5 && jiff::civil::Time::strptime("%H:%M", time).is_ok();
    if !valid_time {
        return Err(usage());
    }
    match (unit, rest) {
        (RepeatUnit::Days, []) => {}
        (RepeatUnit::Weeks, [spec]) => task.days = days(spec)?,
        (RepeatUnit::Months, [day]) => task.day = month_day(day).ok_or_else(usage)?,
        (RepeatUnit::Years, [month, day]) => {
            task.month = MONTHS.iter().position(|name| month.eq_ignore_ascii_case(name)).ok_or_else(usage)? as u8 + 1;
            task.day = month_day(day).ok_or_else(usage)?;
        }
        _ => return Err(usage()),
    }
    task.time = (*time).to_owned();
    Ok(())
}

const MONTHS: [&str; 12] = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const WEEKS: [(&str, WeekOfMonth); 5] = [
    ("first", WeekOfMonth::First),
    ("second", WeekOfMonth::Second),
    ("third", WeekOfMonth::Third),
    ("fourth", WeekOfMonth::Fourth),
    ("last", WeekOfMonth::Last),
];

/// `15`, `last`, `second-tue` or `last-working`.
fn month_day(text: &str) -> Option<MonthDay> {
    let text = text.to_ascii_lowercase();
    if let Ok(date) = text.parse::<u8>() {
        return (1..=31).contains(&date).then_some(MonthDay::Date { date });
    }
    match text.as_str() {
        "last" => return Some(MonthDay::LastDay {}),
        "first-working" => return Some(MonthDay::WorkingDay { which: FirstOrLast::First }),
        "last-working" => return Some(MonthDay::WorkingDay { which: FirstOrLast::Last }),
        _ => {}
    }
    let (week, weekday) = text.split_once('-')?;
    let week = WEEKS.iter().find(|(name, _)| *name == week)?.1;
    Some(MonthDay::Weekday {
        week,
        weekday: Weekday::named(weekday)?,
    })
}

/// A task's moment as `--task-at` takes it: what `show` prints.
fn task_at_text(task: &Task, unit: RepeatUnit) -> String {
    let day = match task.day {
        MonthDay::Date { date } => date.to_string(),
        MonthDay::LastDay {} => "last".into(),
        MonthDay::WorkingDay { which: FirstOrLast::First } => "first-working".into(),
        MonthDay::WorkingDay { which: FirstOrLast::Last } => "last-working".into(),
        MonthDay::Weekday { week, weekday } => {
            let week = WEEKS.iter().find(|(_, each)| *each == week).expect("every week is listed").0;
            format!("{week}-{}", weekday.as_str())
        }
    };
    match unit {
        RepeatUnit::Days => task.time.clone(),
        RepeatUnit::Weeks => format!(
            "{} {}",
            task.days.iter().map(|day| day.as_str()).collect::<Vec<_>>().join(","),
            task.time
        ),
        RepeatUnit::Months => format!("{day} {}", task.time),
        RepeatUnit::Years => format!("{} {day} {}", MONTHS[usize::from(task.month) - 1], task.time),
    }
}

/// `CN`, `DE/BY` or `none`.
fn holidays(text: &str) -> Result<Option<HolidayPlace>, RpcError> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("none") {
        return Ok(None);
    }
    let (country, region) = match text.split_once('/') {
        Some((country, region)) => (country, Some(region.trim().to_owned())),
        None => (text, None),
    };
    Ok(Some(HolidayPlace {
        country: country.trim().to_ascii_uppercase(),
        region,
    }))
}

/// `queue` or `steer`.
fn if_busy(text: &str) -> Result<IfBusy, RpcError> {
    match text.trim().to_ascii_lowercase().as_str() {
        "queue" => Ok(IfBusy::Queue),
        "steer" => Ok(IfBusy::Steer),
        _ => Err(failed(format!("--if-busy {text:?}: give queue or steer"))),
    }
}

async fn user_zone(shard: &Shard) -> Result<String, RpcError> {
    let reported = demi_backend_runners::command_context::reported(&shard.services().control, shard.user())
        .await
        .map_err(failed)?;
    Ok(reported.locale.time_zone)
}

async fn create(shard: Rc<Shard>, call: Call<PartsArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    let zone = user_zone(&shard).await?;
    let mut patch = patch(&shard, call.args, zone, None, TaskOps::default()).await?;
    // The user's grant is the confirmation: it is on at once.
    patch.paused = Some(false);
    let (schedule, secret) = shard.create_schedule(&conversation, patch).await.map_err(refused)?;
    print_shown(&shard, &port, &call.invocation.path.join(" "), call.invocation.json, schedule, secret).await
}

async fn edit(shard: Rc<Shard>, call: Call<EditArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    let id = id_of(&call.args.id)?;
    let current = shard.stored_schedule(&conversation, &id).await.map_err(refused)?;
    let ops = TaskOps {
        task: call.args.task,
        add: call.args.add_task == Some(true),
        delete: call.args.delete_task,
    };
    let mut patch = patch(&shard, call.args.parts, current.time_zone.clone(), Some(&current), ops).await?;
    patch.paused = match (call.args.paused == Some(true), call.args.on == Some(true)) {
        (true, true) => return Err(failed("--paused and --on contradict each other; give one")),
        (true, false) => Some(true),
        (false, true) => Some(false),
        (false, false) => None,
    };
    let new_secret = call.args.new_secret == Some(true);
    if patch == SchedulePatch::default() && !new_secret {
        return Err(failed("nothing to change: give an option, or the message on stdin"));
    }
    let (schedule, secret) = shard.change_schedule(&conversation, &id, patch, new_secret).await.map_err(refused)?;
    print_shown(&shard, &port, &call.invocation.path.join(" "), call.invocation.json, schedule, secret).await
}

async fn run(shard: Rc<Shard>, call: Call<RunArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    let id = id_of(&call.args.id)?;
    let schedule = shard.run_schedule_now(&conversation, &id, call.args.task).await.map_err(refused)?;
    port.stdout(format!("Started a run of \u{201c}{}\u{201d}.\n", schedule.name)).await?;
    Ok(0)
}

async fn delete(shard: Rc<Shard>, call: Call<IdArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let conversation = conversation_of(&call.invocation)?;
    let id = id_of(&call.args.id)?;
    let schedule = shard.stored_schedule(&conversation, &id).await.map_err(refused)?;
    shard.delete_schedule(&conversation, &id).await.map_err(refused)?;
    port.stdout(format!("Deleted the schedule \u{201c}{}\u{201d}.\n", schedule.name)).await?;
    Ok(0)
}
