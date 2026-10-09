//! The `demi shell` command group (`runtime.md` § The whole output,
//! § Stopping a command): `demi shell output` prints a command of the
//! conversation's whole output, as numbered lines a page at a time, the
//! lines a range names, the newest lines, or the bytes as they are, or
//! returns one of the media the command's declared commands returned. It
//! reads a running command's output and media from its Host through the
//! command's job, and an ended command's from the conversation's store.
//! `demi shell stop` stops a running command of the conversation through
//! the shell environment that runs it, as a page's stop does.

use std::rc::Weak;

use bytes::Bytes;
use demi_agent_store::{StoredCommand, StoredOutput};
use demi_host_interface::{MediumKept, StoredMedium};
use demi_agent_tools::{HostResolver, PAGE_CHARS};
use demi_host_interface::{
    Ending, GroupBuilder, LeafBuilder, OutputText, Piece, RpcError, RpcPort, Seen, ShellError,
    Streams, TypedRpc, WholeOutput,
};
use demi_shared_types::{B64Bytes, BlobRef, CommandEnd, CommandId, StreamKind};
use futures_util::FutureExt;
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

const GROUP_SUMMARY: &str = "Shell commands: read a command's whole output, or stop a command.";

/// The group's entry in the model's capability index (`system-prompt.md`
/// § Capability index).
const GROUP_ENTRY: &str = "Prints the whole output of a command you ran earlier, by its commandId, a page at a time, and stops a running command. Use output when a result was cut short in the middle; the result says so with a line naming this command. Use stop to end a dev server, a watcher or any command you no longer need.";

const STOP_SUMMARY: &str = "Stop running commands of this conversation by their commandIds, in order, whichever agent ran them, and wait until each has ended. Its next shell_status shows it aborted, with its last output. Stopping a command that has ended already succeeds, so it is safe to repeat.";

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

/// Stops each of the conversation's commands the call names, in order. A
/// command that cannot be stopped is told on stderr, and the next is
/// stopped all the same.
async fn stop<H: HostResolver>(call: Invoked<H, StopArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let mut failed = false;
    for id in &call.args.id {
        match stop_one(&call.tree, id).await {
            Ok(said) => port.stdout(said.into_bytes()).await?,
            Err(reason) => {
                failed = true;
                fail_one(&port, &call.command, id, &reason).await?;
            }
        }
    }
    Ok(u8::from(failed))
}

/// Stops the conversation's command `id` through the shell environment of
/// the node that runs it, and waits until it has ended: what the command
/// prints of it, or why it could not.
async fn stop_one<H: HostResolver>(tree: &Tree<H>, id: &str) -> Result<String, String> {
    let unknown = || "no such command in this conversation".to_owned();
    let command = CommandId::try_from(id).map_err(|_| unknown())?;
    Ok(match tree.command_place(&command).await? {
        CommandPlace::Unknown => return Err(unknown()),
        CommandPlace::Stored(_) => format!("[command {id} had already ended]\n"),
        CommandPlace::Held(node) => match node.environment_of(&command) {
            Some(environment) if environment.ended(&command).now_or_never().is_none() => {
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
        let ended = match self.end {
            Some(CommandEnd::Exited { exit_code }) => format!(", exit code {exit_code}"),
            Some(CommandEnd::Stopped) => ", stopped".to_owned(),
            Some(CommandEnd::Lost) => ", ended with its Host's connection".to_owned(),
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
