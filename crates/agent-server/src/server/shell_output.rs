//! The `demi shell` command group (`runtime.md` § The whole output):
//! `demi shell output` prints a command of the conversation's whole output,
//! as numbered lines a page at a time, the lines a range names, the newest
//! lines, or the bytes as they are. It reads a running command's output
//! from its Host through the command's job, and an ended command's from the
//! conversation's store.

use std::rc::Weak;

use bytes::Bytes;
use demi_agent_store::{COMMAND_OUTPUT_DAYS, StoredOutput};
use demi_agent_tools::{AgentHarness, PAGE_CHARS};
use demi_shared_types::{CommandId, StreamKind};
use demi_host_interface::{
    GroupBuilder, LeafBuilder, OutputText, Piece, RpcError, RpcPort, Seen, ShellError, Streams,
    TypedRpc, WholeOutput,
};
use schemars::JsonSchema;
use serde::Deserialize;

use super::{
    commands::{Invoked, verb},
    tree::Tree,
};
use crate::AgentServer;

/// The characters of one line a page shows.
const LINE_CHARS: usize = 2_000;

/// The size of the writes `--raw` makes.
const RAW_CHUNK_BYTES: usize = 1024 * 1024;

const GROUP_SUMMARY: &str = "Shell commands: read a command's whole output.";

const OUTPUT_SUMMARY: &str = "Print a command's whole output by its commandId: numbered lines a page at a time, as `cat -n` shows them, from the first line or the lines --lines <from>-<to> names; the newest with --tail <n>. --stdout or --stderr takes one stream, with line numbers of its own. --raw prints the bytes as they are, unnumbered and unpaged, for pipes and files: `grep -n` on it gives the numbers --lines takes (`demi shell output 17 --raw | grep -n FAIL`). Any command of this conversation, running or ended.";

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
}

/// The `shell` group.
pub(super) fn shell_group<H: AgentHarness>(server: Weak<AgentServer<H>>) -> GroupBuilder {
    GroupBuilder::new("shell", GROUP_SUMMARY).leaf(
        LeafBuilder::rpc("output", OUTPUT_SUMMARY)
            .input::<OutputArgs>()
            .positionals(["id"])
            .success_output("the page, the lines, or with --raw the bytes on stdout; with --raw, a line on stderr where bytes were left out")
            .failure_output("\"demi shell output: <reason>\" on stderr, exit 1")
            .bind(TypedRpc::new(verb(server, output))),
    )
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
    running: bool,
}

async fn output<H: AgentHarness>(
    call: Invoked<H, OutputArgs>,
    port: RpcPort,
) -> Result<u8, RpcError> {
    let args = call.args;
    let id = args.id.as_str();
    let streams = match (args.stdout == Some(true), args.stderr == Some(true)) {
        (true, true) => return fail(&port, "--stdout and --stderr do not go together").await,
        (true, false) => Streams::Only(StreamKind::Stdout),
        (false, true) => Streams::Only(StreamKind::Stderr),
        (false, false) => Streams::Both,
    };
    let reading = match (&args.lines, args.tail) {
        (Some(_), Some(_)) => return fail(&port, "--lines and --tail do not go together").await,
        (Some(lines), None) => match range(lines) {
            Some((from, to)) => Reading::Lines { from, to },
            None => {
                let reason = format!("--lines {lines}: lines count from 1, and a range ends at or after its start");
                return fail(&port, &reason).await;
            }
        },
        (None, Some(lines)) => Reading::Tail { lines },
        (None, None) => Reading::Page,
    };
    let raw = args.raw == Some(true);
    if raw && !matches!(reading, Reading::Page) {
        return fail(&port, "--raw prints all of the output; take part of it with sed or tail").await;
    }
    let found = match find(&call.tree, id).await {
        Ok(found) => found,
        Err(reason) => return fail(&port, &reason).await,
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
        return fail(
            &port,
            &format!("the stdout of {id} is binary; save it: demi shell output {id} --raw --stdout > <file>"),
        )
        .await;
    }
    let text = found.output.text(streams, binary, Seen::default());
    let page = Page {
        text: &text,
        id,
        streams,
        running: found.running,
    };
    let last = text.last_line();
    let printed = match reading {
        Reading::Page => page.forward(1, last),
        Reading::Lines { from, to } if from > last => {
            let reason = format!("lines {from}-{to} are past the end: the output has {last} lines");
            return fail(&port, &reason).await;
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
async fn find<H: AgentHarness>(tree: &Tree<H>, id: &str) -> Result<Found, String> {
    let unknown = || format!("no command {id} in this conversation");
    let command = CommandId::try_from(id).map_err(|_| unknown())?;
    if let Some(node) = tree.holder(&command) {
        match node.read_output(&command).await {
            Ok(output) => {
                return Ok(Found {
                    output,
                    running: true,
                });
            }
            // It ended, and its output is stored.
            Err(ShellError::NotRunning(_)) => {}
            Err(error) => return Err(format!("the output of {id} could not be read: {error}")),
        }
    }
    match tree.store().command_output(&command).await {
        Ok(Some(StoredOutput::Stored(output))) => Ok(Found {
            output,
            running: false,
        }),
        Ok(Some(StoredOutput::NotStored(reason))) => {
            Err(format!("the output of {id} was not stored: {reason}"))
        }
        Ok(Some(StoredOutput::Removed(at))) => Err(format!(
            "the output of {id} was removed on {}, {COMMAND_OUTPUT_DAYS} days after the command ended",
            at.to_jiff().strftime("%Y-%m-%d")
        )),
        Ok(None) => Err(unknown()),
        Err(error) => Err(format!("the output of {id} could not be read: {error}")),
    }
}

/// A page of an output's text.
struct Page<'a> {
    text: &'a OutputText,
    id: &'a str,
    streams: Streams,
    running: bool,
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
        let so_far = if self.running { " so far" } else { "" };
        let streams = match self.streams {
            Streams::Both => "stdout and stderr",
            Streams::Only(StreamKind::Stdout) => "stdout",
            Streams::Only(StreamKind::Stderr) => "stderr",
        };
        match shown {
            Some((first, last)) => format!(
                "[command {}: lines {first}-{last} of {of}{so_far}, {streams}]",
                self.id
            ),
            None => format!("[command {}: {of} lines{so_far}, {streams}]", self.id),
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

/// A failure: `demi shell output: <reason>` on stderr, exit 1.
async fn fail(port: &RpcPort, reason: &str) -> Result<u8, RpcError> {
    port.stderr(format!("demi shell output: {reason}\n").into_bytes())
        .await?;
    Ok(1)
}
