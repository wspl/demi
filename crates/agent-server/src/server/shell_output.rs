//! The `demi shell` command group (`runtime.md` § The `demi shell`
//! commands, § The whole output): `demi shell status` looks at commands of
//! the conversation as a result shows them, waits for their end and changes
//! how often they report; `demi shell input` writes to a command's input;
//! `demi shell output` prints a command of the conversation's whole output,
//! as numbered lines a page at a time, the lines a range names, the newest
//! lines, or the bytes as they are, or returns one of the media the
//! command's declared commands returned. It reads a running command's
//! output and media from its Host through the command's job, and an ended
//! command's from the conversation's store. `demi shell stop` stops a
//! running command of the conversation through the shell environment that
//! runs it, as a page's stop does.

use std::{rc::Weak, sync::Arc, time::Duration};

use bytes::Bytes;
use demi_agent_store::{StoredCommand, StoredOutput};
use demi_agent_tools::{
    HostResolver, INTERVAL_CAP_MS, Look, PAGE_CHARS, Stopper, duration, look_text, taken_interval,
    whole_look_text,
};
use demi_host_interface::{
    CommandState, Ending, GroupBuilder, LeafBuilder, OutputText, Piece, RpcError, RpcPort, Seen,
    ShellError, Streams, TypedRpc, WholeOutput,
};
use demi_host_interface::{MediumKept, StoredMedium};
use demi_shared_types::{B64Bytes, BlobRef, CommandEnd, CommandId, NodeId, StreamKind};
use futures_util::{FutureExt, future::join_all};
use schemars::JsonSchema;
use serde::Deserialize;

use super::{
    commands::{Invoked, fail_one, verb},
    tree::{CommandPlace, Tree},
};
use crate::AgentServer;

/// The characters of one line a page shows.
const LINE_CHARS: usize = 2_000;

/// The size of the writes `--raw` makes.
const RAW_CHUNK_BYTES: usize = 1024 * 1024;

const GROUP_SUMMARY: &str = "Shell commands: look at a command, answer its prompt, read its whole output, or stop it.";

/// The group's entry in the model's capability index (`system-prompt.md`
/// § Capability index).
const GROUP_ENTRY: &str = "Works on the commands your shell calls ran, by their commandIds: status looks at a command again, waits for its end and changes how often it reports to you; input answers its prompt; output prints its whole output a page at a time, when a result was cut short in the middle; stop ends a dev server, a watcher or any command you no longer need.";

const STATUS_SUMMARY: &str = "Look at commands of this conversation by their commandIds, in order: each one's status, its exit code once it has ended, how long it has run and printed nothing, and its output since your last look, as a shell result shows them; one that has ended returns its media. --wait <duration>, such as 30s or 5m, waits up to that long for them to end first. --interval <duration> makes them report to you every so long from now on, and --resident only when they end.";

const INPUT_SUMMARY: &str = "Write stdin to a running command's input by its commandId, such as an answer to its prompt, with a newline for a line-based prompt (`demi shell input 17 <<'EOF'`). Prints nothing; a command that is not running fails. Look at what the command did with it with demi shell status.";

const STOP_SUMMARY: &str = "Stop running commands of this conversation by their commandIds, in order, whichever agent ran them, and wait until each has ended. Its next status shows it aborted, with its last output. Stopping a command that has ended already succeeds, so it is safe to repeat.";

const OUTPUT_SUMMARY: &str = "Print a command's whole output by its commandId: numbered lines a page at a time, as `cat -n` shows them, from the first line or the lines --lines <from>-<to> names; the newest with --tail <n>. --stdout or --stderr takes one stream, with line numbers of its own. --raw prints the bytes as they are, unnumbered and unpaged, for pipes and files: `grep -n` on it gives the numbers --lines takes (`demi shell output 17 --raw | grep -n FAIL`). --medium <n> returns the command's medium n, the image or video its line `[medium n: …]` stands for, as it came: shown to you again, or its bytes into a file (`demi shell output 17 --medium 2 > shot.png`). Any command of this conversation, running or ended.";

/// The input of `demi shell output`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OutputArgs {
    /// The command's commandId, as its result names it
    id: String,
    /// The lines to print, as <from>-<to>
    #[schemars(pattern(r"^[0-9]+-[0-9]+$"))]
    lines: Option<String>,
    /// Print the last n lines
    #[schemars(range(min = 1))]
    tail: Option<u64>,
    /// Only stdout
    stdout: Option<bool>,
    /// Only stderr
    stderr: Option<bool>,
    /// The bytes as they are: unnumbered and unpaged
    raw: Option<bool>,
    /// The command's medium n, as it came
    #[schemars(range(min = 1))]
    medium: Option<u32>,
}

/// The input of `demi shell status`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StatusArgs {
    /// The commands' commandIds, as their results name them
    #[schemars(length(min = 1))]
    id: Vec<String>,
    /// Wait up to this long for the commands to end first, such as 30s or 5m
    wait: Option<String>,
    /// Report every so long from now on, such as 5m
    interval: Option<String>,
    /// Report only the commands' end from now on
    resident: Option<bool>,
}

/// The input of `demi shell input`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct InputArgs {
    /// The command's commandId, as its result names it
    id: String,
    /// What to write to the command's input
    input: String,
}

/// The input of `demi shell stop`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct StopArgs {
    /// The commands' commandIds, as their results name them
    #[schemars(length(min = 1))]
    id: Vec<String>,
}

/// The `shell` group.
pub(super) fn shell_group<H: HostResolver>(server: Weak<AgentServer<H>>) -> GroupBuilder {
    GroupBuilder::new("shell", GROUP_SUMMARY)
        .index_entry(GROUP_ENTRY)
        .leaf(
            LeafBuilder::rpc("status", STATUS_SUMMARY)
                .input::<StatusArgs>()
                .positionals(["id"])
                .success_output("each command's status as a shell result shows it, a blank line between two; with --interval or --resident, a line first that says how it reports from now on; an ended command's media")
                .media()
                .failure_output("a line per command that cannot be looked at, \"demi shell status: <id>: <reason>\", on stderr; the others are still shown, and the command exits 1")
                .bind(TypedRpc::new(verb(server.clone(), status))),
        )
        .leaf(
            LeafBuilder::rpc("input", INPUT_SUMMARY)
                .input::<InputArgs>()
                .positionals(["id"])
                .stdin_field("input")
                .success_output("nothing")
                .failure_output("\"demi shell input: <reason>\" on stderr, exit 1")
                .bind(TypedRpc::new(verb(server.clone(), input))),
        )
        .leaf(
            LeafBuilder::rpc("output", OUTPUT_SUMMARY)
                .input::<OutputArgs>()
                .positionals(["id"])
                .success_output("the page, the lines, or with --raw the bytes on stdout; with --raw, a line on stderr where bytes were left out; with --medium, the medium")
                .media()
                .failure_output("\"demi shell output: <reason>\" on stderr, exit 1")
                .bind(TypedRpc::new(verb(server.clone(), output))),
        )
        .leaf(
            LeafBuilder::rpc("stop", STOP_SUMMARY)
                .input::<StopArgs>()
                .positionals(["id"])
                .success_output("\"[command <id> stopped]\", or \"[command <id> had already ended]\" for a command that had ended")
                .failure_output("a line per command that cannot be stopped, \"demi shell stop: <id>: <reason>\", on stderr; the others are still stopped, and the command exits 1")
                .bind(TypedRpc::new(verb(server, stop))),
        )
}

/// How a look changes how a command reports: every so many milliseconds,
/// or only its end when none.
type IntervalChange = Option<u32>;

/// Looks at each of the conversation's commands the call names, in order,
/// once they ended or `--wait` passed; changes first how each reports when
/// the call says so. A command that cannot be looked at is told on stderr,
/// and the next is looked at all the same.
async fn status<H: HostResolver>(
    call: Invoked<H, StatusArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let args = &call.args;
    let mut notes = Vec::new();
    let change = match (&args.interval, args.resident == Some(true)) {
        (Some(_), true) => return fail("--interval and --resident do not go together"),
        (Some(text), false) => {
            let asked = match milliseconds(text) {
                Ok(asked) => asked,
                Err(reason) => return fail(&format!("--interval {text}: {reason}")),
            };
            let floor = call.tree.interval_floor_ms();
            let (taken, outside) = taken_interval(asked, floor, "--interval");
            if outside.is_some() {
                let bound = if taken == INTERVAL_CAP_MS { "above the cap" } else { "below the floor" };
                notes.push(format!("--interval {text} is {bound}; taken as {}", duration(taken.into())));
            }
            Some(Some(taken))
        }
        (None, true) => Some(None),
        (None, false) => None,
    };
    // The caller looks at the commands from now on: their end reports end
    // no window of its calls, since this look shows their ends itself.
    let looking = call.tree.node(&call.caller);
    let _look = looking.as_ref().map(|node| {
        node.look_at(args.id.iter().filter_map(|id| CommandId::try_from(id.as_str()).ok()).collect())
    });
    if let Some(text) = &args.wait {
        let wait = match milliseconds(text) {
            Ok(wait) => Duration::from_millis(wait),
            Err(reason) => return fail(&format!("--wait {text}: {reason}")),
        };
        wait_for_ends(&call.tree, &args.id, wait).await;
    }
    let mut failed = false;
    let mut shown = Vec::new();
    for id in &args.id {
        match status_one(&call.tree, &call.caller, id, change).await {
            Ok((text, media)) => {
                shown.push(text);
                for blob in media {
                    port.medium(blob).await?;
                }
            }
            Err(reason) => {
                failed = true;
                fail_one(&port, &call.command, id, &reason).await?;
            }
        }
    }
    let printed: Vec<String> = notes.into_iter().chain([shown.join("\n\n")]).collect();
    port.stdout(format!("{}\n", printed.join("\n")).into_bytes()).await?;
    Ok(u8::from(failed))
}

/// A duration as `--wait` and `--interval` take it, such as `30s`, `5m`
/// or `1h30m`, in whole milliseconds from 1.
fn milliseconds(text: &str) -> Result<u64, String> {
    let parsed: jiff::SignedDuration = text
        .parse()
        .map_err(|_| "a duration such as 30s, 5m or 1h30m".to_owned())?;
    match u64::try_from(parsed.as_millis()) {
        Ok(milliseconds) if milliseconds > 0 => Ok(milliseconds),
        _ => Err("a duration longer than none".to_owned()),
    }
}

/// Waits until each of the commands `ids` names that a node's shells run
/// has ended, or `wait` has passed.
async fn wait_for_ends<H: HostResolver>(tree: &Tree<H>, ids: &[String], wait: Duration) {
    let environments: Vec<_> = ids
        .iter()
        .filter_map(|id| CommandId::try_from(id.as_str()).ok())
        .filter_map(|command| {
            let environment = tree.holder(&command)?.environment_of(&command)?;
            Some((command, environment))
        })
        .collect();
    let ends = join_all(
        environments
            .iter()
            .map(|(command, environment)| environment.ended(command)),
    );
    // The time passing ends the wait as their ends do.
    let _ = tokio::time::timeout(wait, ends).await;
}

/// What a look at the conversation's command `id` shows the node `caller`,
/// with the stored media of a command whose end it shows; first, `change`
/// changes how the command reports. The node that runs the command looks
/// from its place in the command's record; any other node, and a node
/// whose command no shells hold any more, from its own place in the
/// command's whole output: each look shows the output since the node's own
/// last look and moves only its place (`runtime.md` § The `demi shell`
/// commands).
async fn status_one<H: HostResolver>(
    tree: &Tree<H>,
    caller: &NodeId,
    id: &str,
    change: Option<IntervalChange>,
) -> Result<(String, Vec<BlobRef>), String> {
    let unknown = || "no such command in this conversation".to_owned();
    let command = CommandId::try_from(id).map_err(|_| unknown())?;
    let looking = tree
        .node(caller)
        .ok_or_else(|| "the calling agent is not live".to_owned())?;
    let mut lines = Vec::new();
    let held = match tree.command_place(&command).await? {
        CommandPlace::Unknown => return Err(unknown()),
        CommandPlace::Stored => None,
        CommandPlace::Held(node) => Some(node),
    };
    if let Some(interval) = change {
        let changed = held
            .as_ref()
            .is_some_and(|node| node.session().set_interval(&command, interval));
        let said = match (changed, interval) {
            (true, Some(interval)) => {
                format!("[command {id} reports every {} from now on]", duration(interval.into()))
            }
            (true, None) => format!("[command {id} reports only its end from now on]"),
            // Its end was reported or shown already.
            (false, _) => format!("[command {id} reports nothing more]"),
        };
        lines.push(said);
    }
    let interval_ms = held
        .as_ref()
        .and_then(|node| node.session().interval_of(&command));
    let look = Look {
        interval_ms,
        ..Look::default()
    };
    if let Some(node) = held.as_ref().filter(|node| node.id() == caller) {
        let environment = node
            .environment_of(&command)
            .ok_or_else(|| format!("command {id} is no longer held"))?;
        let status = environment.status(&command).map_err(|error| error.to_string())?;
        lines.push(look_text(&status, look));
        let mut media = Vec::new();
        if !matches!(status.state, CommandState::Running { .. }) {
            node.saw_end(&command).await;
            media = stored_media(tree, &command).await;
        }
        return Ok((lines.join("\n"), media));
    }
    let running = held.as_ref().and_then(|node| {
        let environment = node.environment_of(&command)?;
        environment
            .ended(&command)
            .now_or_never()
            .is_none()
            .then_some((node, environment))
    });
    let place = looking.place(&command);
    let shown = match running {
        Some((node, environment)) => {
            let whole = environment
                .read_output(&command)
                .await
                .map_err(|error| format!("the output of {id} could not be read: {error}"))?;
            let running_ms = node
                .live_views()
                .into_iter()
                .find(|view| view.command_id == command)
                .map_or(0, |view| view.running_ms);
            let idle_ms = environment
                .quiet(&command)
                .map_or(0, |quiet| u64::try_from(quiet.as_millis()).unwrap_or(u64::MAX));
            let state = CommandState::Running { hint: None };
            looking.set_place(&command, whole.seen_through());
            whole_look_text(&command, state, running_ms, idle_ms, Arc::new(whole), place, look)
        }
        None => match tree.store().command_output(&command).await {
            Ok(Some(StoredCommand {
                end,
                output: StoredOutput::Stored { output, .. },
            })) => {
                let state = match end {
                    CommandEnd::Exited { exit_code } => CommandState::Exited {
                        exit_code,
                        binary_stdout: None,
                        media: Vec::new(),
                    },
                    CommandEnd::Stopped => CommandState::Aborted,
                    CommandEnd::Lost { .. } | CommandEnd::Unrecorded => {
                        lines.extend(ended_lines(id, end));
                        return Ok((lines.join("\n"), Vec::new()));
                    }
                };
                looking.set_place(&command, Seen::ALL);
                whole_look_text(&command, state, 0, 0, Arc::new(output), place, look)
            }
            Ok(Some(StoredCommand { end, .. })) => ended_lines(id, end).join("\n"),
            Ok(None) => ended_lines(id, CommandEnd::Unrecorded).join("\n"),
            Err(error) => return Err(format!("the output of {id} could not be read: {error}")),
        },
    };
    lines.push(shown);
    Ok((lines.join("\n"), Vec::new()))
}

/// The media the conversation stored of `command`, which ended, by number.
async fn stored_media<H: HostResolver>(tree: &Tree<H>, command: &CommandId) -> Vec<BlobRef> {
    let Ok(Some(StoredCommand {
        output: StoredOutput::Stored { media, .. },
        ..
    })) = tree.store().command_output(command).await
    else {
        return Vec::new();
    };
    media
        .into_iter()
        .filter_map(|medium| match medium.kept {
            MediumKept::Stored { blob } => Some(blob),
            MediumKept::Missing { .. } => None,
        })
        .collect()
}

/// What a look shows of a command whose output the conversation does not
/// hold, or whose end keeps no status: how it ended, and where its output
/// would be.
fn ended_lines(id: &str, end: CommandEnd) -> Vec<String> {
    let mut lines = match end {
        CommandEnd::Exited { exit_code } => {
            vec!["status: exited".to_owned(), format!("exitCode: {exit_code}")]
        }
        CommandEnd::Stopped => vec!["status: aborted".to_owned()],
        CommandEnd::Lost { reason } => vec![format!("status: lost: {reason}")],
        CommandEnd::Unrecorded => vec!["status: ended".to_owned()],
    };
    lines.push(format!("commandId: {id}"));
    lines.push(format!("[its whole output: demi shell output {id}]"));
    lines
}

/// Writes the call's input to the conversation's command it names, through
/// the shell environment that runs it.
async fn input<H: HostResolver>(call: Invoked<H, InputArgs>, _port: RpcPort) -> Result<u8, RpcError> {
    let id = &call.args.id;
    let unknown = || format!("no command {id} in this conversation");
    let Ok(command) = CommandId::try_from(id.as_str()) else {
        return fail(&unknown());
    };
    let place = match call.tree.command_place(&command).await {
        Ok(place) => place,
        Err(reason) => return fail(&reason),
    };
    let environment = match place {
        CommandPlace::Unknown => return fail(&unknown()),
        CommandPlace::Stored => None,
        CommandPlace::Held(node) => node.environment_of(&command),
    };
    let Some(environment) = environment else {
        return fail(&format!("command {id} is not running"));
    };
    match environment.write(&command, Bytes::from(call.args.input)).await {
        Ok(()) => Ok(0),
        Err(ShellError::NotRunning(_)) => fail(&format!("command {id} is not running")),
        Err(ShellError::EmptyStdin) => fail("the input is empty"),
        Err(error) => fail(&error.to_string()),
    }
}

/// Stops each of the conversation's commands the call names, in order. A
/// command that cannot be stopped is told on stderr, and the next is
/// stopped all the same.
async fn stop<H: HostResolver>(call: Invoked<H, StopArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let mut failed = false;
    for id in &call.args.id {
        match stop_one(&call.tree, &call.caller, id).await {
            Ok(said) => port.stdout(said.into_bytes()).await?,
            Err(reason) => {
                failed = true;
                fail_one(&port, &call.command, id, &reason).await?;
            }
        }
    }
    Ok(u8::from(failed))
}

/// Stops the conversation's command `id` for the node `caller` through the
/// shell environment of the node that runs it, and waits until it has
/// ended: what the command prints of it, or why it could not. The end's
/// report says which agent stopped it, and the node that stopped its own
/// command, which it was told so, hears nothing more of it.
async fn stop_one<H: HostResolver>(tree: &Tree<H>, caller: &NodeId, id: &str) -> Result<String, String> {
    let unknown = || "no such command in this conversation".to_owned();
    let command = CommandId::try_from(id).map_err(|_| unknown())?;
    Ok(match tree.command_place(&command).await? {
        CommandPlace::Unknown => return Err(unknown()),
        CommandPlace::Stored => format!("[command {id} had already ended]\n"),
        CommandPlace::Held(node) => match node.environment_of(&command) {
            Some(environment) if environment.ended(&command).now_or_never().is_none() => {
                let stopper = if node.id() == caller {
                    Some(Stopper::Itself)
                } else {
                    tree.node(caller)
                        .map(|stopping| Stopper::Agent(stopping.record().number))
                };
                if let Some(stopper) = stopper {
                    node.stopped_by(&command, stopper);
                }
                // The environment that runs it stops it, as for a page's
                // stop; no Host is resolved, so the call never enters the
                // conversation's file gate its own job holds a lease of.
                let stopped = match environment.abort(&command).await {
                    Ok(()) => environment.ended(&command).await,
                    Err(error) => Err(error),
                };
                match stopped.map_err(|error| error.to_string())? {
                    Ending::Aborted => format!("[command {id} stopped]\n"),
                    // It ended by itself before the stop reached it.
                    Ending::Exited(_) => format!("[command {id} had already ended]\n"),
                }
            }
            _ => format!("[command {id} had already ended]\n"),
        },
    })
}

/// What a reading prints of the output's lines.
enum Reading {
    /// A page from the first line.
    Page,
    /// A page from `from`, to `to` at most.
    Lines { from: u64, to: u64 },
    /// The last `lines` lines, or as many as fit.
    Tail { lines: u64 },
}

/// An output found, and whether its command still runs.
struct Found {
    output: WholeOutput,
    /// How the command ended; none while it runs.
    end: Option<CommandEnd>,
}

async fn output<H: HostResolver>(
    call: Invoked<H, OutputArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let args = call.args;
    let id = args.id.as_str();
    if let Some(number) = args.medium {
        let alone = args.lines.is_none()
            && args.tail.is_none()
            && args.stdout.is_none()
            && args.stderr.is_none()
            && args.raw.is_none();
        if !alone {
            return fail("--medium returns one medium and goes with no other option");
        }
        return match find_medium(&call.tree, id, number).await {
            Ok(blob) => {
                port.medium(blob).await?;
                Ok(0)
            }
            Err(reason) => fail(&reason),
        };
    }
    let streams = match (args.stdout == Some(true), args.stderr == Some(true)) {
        (true, true) => return fail("--stdout and --stderr do not go together"),
        (true, false) => Streams::Only(StreamKind::Stdout),
        (false, true) => Streams::Only(StreamKind::Stderr),
        (false, false) => Streams::Both,
    };
    let reading = match (&args.lines, args.tail) {
        (Some(_), Some(_)) => return fail("--lines and --tail do not go together"),
        (Some(lines), None) => match range(lines) {
            Some((from, to)) => Reading::Lines { from, to },
            None => {
                let reason = format!(
                    "--lines {lines}: lines count from 1, and a range ends at or after its start"
                );
                return fail(&reason);
            }
        },
        (None, Some(lines)) => Reading::Tail { lines },
        (None, None) => Reading::Page,
    };
    let raw = args.raw == Some(true);
    if raw && !matches!(reading, Reading::Page) {
        return fail("--raw prints all of the output; take part of it with sed or tail");
    }
    let found = match find(&call.tree, id).await {
        Ok(found) => found,
        Err(reason) => return fail(&reason),
    };
    if raw {
        let text = found.output.text(streams, None, Seen::default());
        for chunk in text.bytes().chunks(RAW_CHUNK_BYTES) {
            port.stdout(Bytes::copy_from_slice(chunk)).await?;
        }
        for note in text.notes() {
            port.stderr(format!("{note}\n").into_bytes()).await?;
        }
        return Ok(0);
    }
    let binary = found.output.binary_stdout_length();
    if binary.is_some() && streams == Streams::Only(StreamKind::Stdout) {
        return fail(&format!("the stdout of {id} is binary; save it: demi shell output {id} --raw --stdout > <file>"));
    }
    let text = found.output.text(streams, binary, Seen::default());
    let page = Page {
        text: &text,
        id,
        streams,
        end: found.end,
    };
    let last = text.last_line();
    let printed = match reading {
        Reading::Page => page.forward(1, last),
        Reading::Lines { from, to } if from > last => {
            let reason = format!("lines {from}-{to} are past the end: the output has {last} lines");
            return fail(&reason);
        }
        Reading::Lines { from, to } => page.forward(from, to.min(last)),
        Reading::Tail { lines } => page.tail(lines),
    };
    port.stdout(printed.into_bytes()).await?;
    Ok(0)
}

/// The lines `<from>-<to>` names, from the first line on and in order.
fn range(lines: &str) -> Option<(u64, u64)> {
    let (from, to) = lines.split_once('-')?;
    let (from, to) = (from.parse().ok()?, to.parse().ok()?);
    (1 <= from && from <= to).then_some((from, to))
}

/// The output of the conversation's command `id`: what its Host kept so far
/// while it runs, the stored output once it ended.
async fn find<H: HostResolver>(tree: &Tree<H>, id: &str) -> Result<Found, String> {
    let unknown = || format!("no command {id} in this conversation");
    let command = CommandId::try_from(id).map_err(|_| unknown())?;
    if let Some(node) = tree.holder(&command) {
        match node.read_output(&command).await {
            Ok(output) => {
                return Ok(Found { output, end: None });
            }
            // It ended, and its output is stored.
            Err(ShellError::NotRunning(_)) => {}
            Err(error) => return Err(format!("the output of {id} could not be read: {error}")),
        }
    }
    match tree.store().command_output(&command).await {
        Ok(Some(StoredCommand {
            end,
            output: StoredOutput::Stored { output, .. },
        })) => Ok(Found {
            output,
            end: Some(end),
        }),
        Ok(Some(StoredCommand {
            output: StoredOutput::NotStored(reason),
            ..
        })) => {
            Err(format!("the output of {id} was not stored: {reason}"))
        }
        Ok(None) => Err(unknown()),
        Err(error) => Err(format!("the output of {id} could not be read: {error}")),
    }
}

/// The blob of medium `number` of the conversation's command `id`: read
/// from its Host and put while the command runs, stored once it ended.
async fn find_medium<H: HostResolver>(
    tree: &Tree<H>,
    id: &str,
    number: u32,
) -> Result<BlobRef, String> {
    let unknown = || format!("no command {id} in this conversation");
    let command = CommandId::try_from(id).map_err(|_| unknown())?;
    let unread = |error: &dyn std::fmt::Display| {
        format!("medium {number} of {id} could not be read: {error}")
    };
    if let Some(node) = tree.holder(&command) {
        match node.read_medium(&command, number).await {
            Ok(bytes) => {
                let blobs = tree.store().session_store(node.id());
                return blobs
                    .blobs()
                    .put(B64Bytes::new(bytes))
                    .await
                    .map_err(|error| unread(&error));
            }
            // It ended, and its media are stored with its output.
            Err(ShellError::NotRunning(_)) => {}
            Err(error @ ShellError::NoMedium { .. }) => return Err(error.to_string()),
            Err(error) => return Err(unread(&error)),
        }
    }
    let media = match tree.store().command_output(&command).await {
        Ok(Some(StoredCommand {
            output: StoredOutput::Stored { media, .. },
            ..
        })) => media,
        Ok(Some(StoredCommand {
            output: StoredOutput::NotStored(reason),
            ..
        })) => {
            return Err(format!("the output of {id} was not stored: {reason}"));
        }
        Ok(None) => return Err(unknown()),
        Err(error) => return Err(format!("the output of {id} could not be read: {error}")),
    };
    let returned = media.len();
    match media.into_iter().find(|medium| medium.number == number) {
        Some(StoredMedium {
            kept: MediumKept::Stored { blob },
            ..
        }) => Ok(blob),
        Some(StoredMedium {
            kept: MediumKept::Missing { reason },
            ..
        }) => Err(format!("medium {number} of {id} is not kept: {reason}")),
        None => Err(ShellError::NoMedium {
            command,
            number,
            returned,
        }
        .to_string()),
    }
}

/// A page of an output's text.
struct Page<'a> {
    text: &'a OutputText,
    id: &'a str,
    streams: Streams,
    /// How the command ended; none while it runs.
    end: Option<CommandEnd>,
}

impl Page<'_> {
    /// The lines from `from` through `to`, as many as fit, with the header
    /// and, when lines of the range remain, the command for the next page.
    fn forward(&self, from: u64, to: u64) -> String {
        let budget = PAGE_CHARS - self.reserved(to);
        let mut lines = Vec::new();
        let mut used = 0;
        let mut shown = None;
        let mut next = None;
        for piece in self.text.forward(from) {
            let number = match piece {
                Piece::Line { number, .. } if number > to => break,
                Piece::Line { number, .. } => Some(number),
                Piece::Note(_) => None,
            };
            let rendered = self.render(&piece);
            let cost = chars(&rendered);
            if used + cost > budget && !lines.is_empty() {
                next = number;
                break;
            }
            used += cost;
            lines.extend(rendered);
            if let Some(number) = number {
                shown = Some(shown.map_or((number, number), |(first, _)| (first, number)));
            }
        }
        let mut page = vec![self.header(shown)];
        page.extend(lines);
        if let Some(next) = next {
            page.push(format!(
                "[next: demi shell output {} --lines {next}-{to}{}]",
                self.id,
                self.flags()
            ));
        }
        page.join("\n") + "\n"
    }

    /// The last `count` lines, as many of them as fit, with the header.
    fn tail(&self, count: u64) -> String {
        let last = self.text.last_line();
        let first = last.saturating_sub(count.saturating_sub(1)).max(1);
        let budget = PAGE_CHARS - self.reserved(last);
        let mut lines = Vec::new();
        let mut used = 0;
        let mut shown = None;
        for piece in self.text.backward() {
            let number = match piece {
                Piece::Line { number, .. } if number < first => break,
                Piece::Line { number, .. } => Some(number),
                Piece::Note(_) => None,
            };
            let rendered = self.render(&piece);
            let cost = chars(&rendered);
            if used + cost > budget && !lines.is_empty() {
                break;
            }
            used += cost;
            lines.push(rendered);
            if let Some(number) = number {
                shown = Some(shown.map_or((number, number), |(_, last)| (number, last)));
            }
        }
        lines.reverse();
        let mut page = vec![self.header(shown)];
        page.extend(lines.into_iter().flatten());
        page.join("\n") + "\n"
    }

    /// A piece as the page shows it: a line numbered as `cat -n` numbers
    /// it, its first characters and a note when it is long, or a note.
    fn render(&self, piece: &Piece<'_>) -> Vec<String> {
        let Piece::Line { number, .. } = piece else {
            return vec![piece.text()];
        };
        let text = piece.text();
        let length = text.chars().count();
        if length <= LINE_CHARS {
            return vec![format!("{number:>6}\t{text}")];
        }
        let start: String = text.chars().take(LINE_CHARS).collect();
        vec![
            format!("{number:>6}\t{start}"),
            format!(
                "[line {number} is {length} characters; whole: demi shell output {} --raw{} | sed -n {number}p]",
                self.id,
                self.flags()
            ),
        ]
    }

    /// What the page holds: its lines, of how many, and of which streams.
    fn header(&self, shown: Option<(u64, u64)>) -> String {
        let of = self.text.last_line();
        let so_far = if self.end.is_none() { " so far" } else { "" };
        let streams = match self.streams {
            Streams::Both => "stdout and stderr",
            Streams::Only(StreamKind::Stdout) => "stdout",
            Streams::Only(StreamKind::Stderr) => "stderr",
        };
        // How it ended, as its record keeps it; a record of a release that
        // kept no end says nothing.
        let ended = match &self.end {
            Some(CommandEnd::Exited { exit_code }) => format!(", exit code {exit_code}"),
            Some(CommandEnd::Stopped) => ", stopped".to_owned(),
            Some(CommandEnd::Lost { reason }) => format!(", lost: {reason}"),
            Some(CommandEnd::Unrecorded) | None => String::new(),
        };
        match shown {
            Some((first, last)) => format!(
                "[command {}: lines {first}-{last} of {of}{so_far}, {streams}{ended}]",
                self.id
            ),
            None => format!("[command {}: {of} lines{so_far}, {streams}{ended}]", self.id),
        }
    }

    /// The characters the header and the next page's command take at most
    /// on a page that ends by line `to`, with their newlines.
    fn reserved(&self, to: u64) -> usize {
        let header = self.header(Some((to, to)));
        let next = format!(
            "[next: demi shell output {} --lines {to}-{to}{}]",
            self.id,
            self.flags()
        );
        header.chars().count() + next.chars().count() + 2
    }

    /// The flags that select the page's streams.
    fn flags(&self) -> &'static str {
        match self.streams {
            Streams::Both => "",
            Streams::Only(StreamKind::Stdout) => " --stdout",
            Streams::Only(StreamKind::Stderr) => " --stderr",
        }
    }
}

/// The characters `lines` take, each with its newline.
fn chars(lines: &[String]) -> usize {
    lines.iter().map(|line| line.chars().count() + 1).sum()
}

/// A failure, which the dispatcher tells after the command's path:
/// `demi shell output: <reason>` on stderr, exit 1.
fn fail(reason: &str) -> Result<u8, RpcError> {
    Err(RpcError::Failed(reason.to_owned()))
}
