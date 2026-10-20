//! `demi conversation history` (`compaction.md` § Reading the earlier
//! history): an agent finds and reads its own transcript, which keeps every
//! block a compaction summarized, from the conversation's database. `read`
//! prints a range of blocks as plain text, each message and text whole and
//! every other block as one line that names how to open it; `open` prints
//! what such a line names, or saves an attachment under the scratch
//! directory. The commands reach no Host, but for the one write of a saved
//! attachment, and need no permission. A block is named by its index in the
//! agent's transcript, written bare: a word that starts with `#` is a
//! comment to a shell.

use std::ops::{Range, RangeInclusive};
use std::rc::{Rc, Weak};

use demi_agent_tools::duration;
use demi_backend_database::search::{HistoryBound, HistoryQuery, SearchKind, occurrences, searchable_text};
use demi_backend_database::tree::{self, Transcript};
use demi_backend_host_access::host_commands::conversation_of;
use demi_backend_host_access::{HostShard, root_of};
use demi_host_interface::{
    Call, FileContents, GroupBuilder, Host as _, LeafBuilder, RpcError, RpcPort, TypedRpc, WhenExists, WriteOptions,
};
use demi_shared_types::{
    AgentMessageEvent, Block, DocumentSource, MediaSource, NodeId, ShellViewStatus, Timestamp, ToolCallBlock,
    ToolView, UserContentBlock,
    output::{self, Cell, Table},
};
use demi_web_api_protocol::ids::{ConversationId, DeviceId};
use jiff::civil::Date;
use jiff::tz::TimeZone;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::organize::{failed, refused, verb};
use crate::shard::Shard;

/// About how many characters a read prints, and a page of what `open`
/// prints of a text.
const BUDGET: usize = 8_000;

/// How many blocks a read takes from the database at a time.
const BATCH: usize = 64;

const GROUP_SUMMARY: &str = "This agent's own transcript, which keeps every block, also what a compaction summarized: search it, read a range of it, open a command, a thinking block, a subagent's report or an attachment it names. A block is named by its index, such as 4213.";

const SEARCH_SUMMARY: &str = "Find your messages, your user's or parent's messages and steers, your texts and the titles of the commands you ran that hold every word, or any with --any; a word matches anywhere in a text, in any language, case aside. The newest come first, five to a page, each a row with its index, who wrote it, when and its text cut around the words, which are marked **like this**, between rows of the message before and after it; stderr names the page and the search of the next. Thinking, scripts and command output are not searched. Filters narrow the search, so a common word does not bury what was asked: --from user keeps your user's or parent's messages and steers, --from agent your texts and commands; --after and --before keep what lies after or before a block, 4213, or a day, 2026-10-14, in the user's time zone, and both may be given; --oldest lists the oldest first. For example, search --from user --before 2026-10-15 README finds what the user asked of the README in the days before.";

const READ_SUMMARY: &str = "Print the transcript in a range of block indices, 4213..4260, 4213.. or ..4260: each message and each of your texts whole, every other block as one line saying what it is and how to open it, such as [4214 command: Run the tests · exit 0 · 3s]; thinking and context blocks are left out. With --from user only your user's or parent's messages are printed, with --from agent only your texts and command lines, as search --from counts them. A read stops at about 8,000 characters, at a block's end, and its last line says where to go on.";

const OPEN_SUMMARY: &str = "Print what a read names in one line, by its index: a command's script, exit code, duration and output, the output a page at a time as demi shell output pages it; a thinking block's text; a subagent's report; a compaction's summary. An attachment or an image of a message, such as 4213/a1, is saved under $DEMI_SCRATCH and its path printed, for demi file view or the ordinary tools. --page <n> takes the next page.";

/// Whose texts a search or a read keeps: the agent's user's or parent's,
/// or the agent's own.
#[derive(Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
enum Side {
    User,
    Agent,
}

impl Side {
    fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Agent => "agent",
        }
    }
}

/// The input of `demi conversation history search`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SearchArgs {
    /// The words to find
    #[schemars(length(min = 1))]
    words: Vec<String>,
    /// Find the texts that hold any of the words, not only every one
    any: Option<bool>,
    /// Keep the messages of your user or parent, user, or your own texts
    /// and commands, agent
    from: Option<Side>,
    /// Keep what lies after a block, such as 4213, or a day, such as
    /// 2026-10-14, in the user's time zone
    after: Option<String>,
    /// Keep what lies before a block, such as 4213, or a day, such as
    /// 2026-10-14, in the user's time zone
    before: Option<String>,
    /// List the oldest first
    oldest: Option<bool>,
    /// The page of hits, from 1
    #[schemars(range(min = 1))]
    page: Option<u64>,
}

/// The input of `demi conversation history read`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ReadArgs {
    /// The blocks to read, by index: 4213..4260, 4213.. or ..4260
    range: String,
    /// Print only the messages of your user or parent, user, or only your
    /// own texts and command lines, agent
    from: Option<Side>,
}

/// The input of `demi conversation history open`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OpenArgs {
    /// What to open, as a read names it: a block's index, such as 4214, or
    /// a message's attachment, such as 4213/a1
    item: String,
    /// The page to print, from 1
    #[schemars(range(min = 1))]
    page: Option<u64>,
}

/// The `history` group of `demi conversation`, whose handlers read in
/// `shard`.
pub(crate) fn history_group(shard: Weak<Shard>) -> GroupBuilder {
    GroupBuilder::new("history", GROUP_SUMMARY)
        .leaf(
            LeafBuilder::rpc("search", SEARCH_SUMMARY)
                .input::<SearchArgs>()
                .positionals(["words"])
                .success_output("a table of the page's hits, each with the message before and after it; on stderr, the page and the search of the next page")
                .json_output::<Vec<SearchRow>>()
                .bind(TypedRpc::new(verb(shard.clone(), search))),
        )
        .leaf(
            LeafBuilder::rpc("read", READ_SUMMARY)
                .input::<ReadArgs>()
                .positionals(["range"])
                .success_output("the blocks of the range as plain text; when it stopped early, a last line \"Stopped at <index> · next: <the read that goes on>\"")
                .bind(TypedRpc::new(verb(shard.clone(), read))),
        )
        .leaf(
            LeafBuilder::rpc("open", OPEN_SUMMARY)
                .input::<OpenArgs>()
                .positionals(["item"])
                .success_output("what the item holds, or the path an attachment was saved to; when more pages follow, a last line naming the next")
                .bind(TypedRpc::new(verb(shard, open))),
        )
}

/// The transcript a call reads: its agent's, in its conversation.
struct Reader {
    conversation: ConversationId,
    node: NodeId,
    /// Whether the agent is the root, whose user is the user; a subagent's
    /// is its parent.
    root: bool,
    zone: TimeZone,
}

impl Reader {
    fn of<A>(call: &Call<A>) -> Result<Self, RpcError> {
        let conversation = conversation_of(&call.invocation)?;
        let caller = call
            .invocation
            .caller
            .as_ref()
            .ok_or_else(|| failed("only an agent's command has a transcript to read"))?;
        // A zone the user's page reported that this build does not know
        // shows the times in UTC rather than refusing the read.
        let zone = TimeZone::get(&call.invocation.context.locale.time_zone).unwrap_or(TimeZone::UTC);
        Ok(Self {
            root: caller.node == root_of(&conversation),
            node: caller.node.clone(),
            conversation,
            zone,
        })
    }

    fn author(&self) -> &'static str {
        if self.root { "user" } else { "parent" }
    }

    /// The bound `text` names of a search, `after` or before it: a block's
    /// index, or a day in the user's time zone, which a search after it
    /// keeps from the next day's start on and before it up to its own
    /// start.
    fn bound(&self, text: &str, after: bool) -> Result<(When, HistoryBound), RpcError> {
        if let Some(at) = index(text) {
            return Ok((When::Index(at), HistoryBound::Index(at)));
        }
        let invalid = || refused(format!("{text}: a block's index, such as 4213, or a day, such as 2026-10-14"));
        let day: Date = text.parse().map_err(|_| invalid())?;
        let start = if after { day.tomorrow().map_err(|_| invalid())? } else { day };
        let moment = start.to_zoned(self.zone.clone()).map_err(|_| invalid())?.timestamp();
        Ok((When::Day(day), HistoryBound::Time(Timestamp::truncate(moment))))
    }

    /// A time as a read shows it: an instant in UTC, as every command
    /// prints one (`commands.md` § Output).
    fn when(&self, time: Timestamp) -> String {
        output::instant(time)
    }

    /// The blocks `range` names of the transcript, and how many it has;
    /// fewer blocks than the range names when the transcript ends first.
    async fn blocks(
        &self,
        shard: &Shard,
        range: RangeInclusive<usize>,
    ) -> Result<(Vec<Block>, usize), RpcError> {
        let node = self.node.clone();
        let read = shard
            .services()
            .conversations
            .read(&self.conversation, move |connection| {
                let length = Transcript::Agent(node.clone()).length(connection)?.unwrap_or(0);
                let end = (*range.end() + 1).min(length);
                let start = (*range.start()).min(end);
                Ok((tree::agent_blocks(connection, &node, start..end)?, length))
            })
            .await
            .map_err(failed)?;
        Ok(read.unwrap_or_default())
    }
}

/// A bound of a search as its options name it again.
enum When {
    Index(usize),
    Day(Date),
}

impl std::fmt::Display for When {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Index(at) => write!(formatter, "{at}"),
            Self::Day(day) => write!(formatter, "{day}"),
        }
    }
}

/// How many hits a page of a search shows.
const HITS_PER_PAGE: usize = 5;

/// How many blocks on each side of a hit a search reads to find the
/// messages before and after it.
const AROUND: usize = 8;

/// The characters a hit shows of its own text, and of each message beside
/// it: about 600 in all.
const HIT_CHARS: usize = 400;
const BESIDE_CHARS: usize = 100;

async fn search(shard: Rc<Shard>, call: Call<SearchArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let reader = Reader::of(&call)?;
    let mut words: Vec<String> = Vec::new();
    for word in call.args.words.iter().flat_map(|words| words.split_whitespace()) {
        if !words.iter().any(|seen| seen == word) {
            words.push(word.to_owned());
        }
    }
    if words.is_empty() {
        return Err(refused("no words to find"));
    }
    let any = call.args.any == Some(true);
    let oldest = call.args.oldest == Some(true);
    let after = call.args.after.as_deref().map(|text| reader.bound(text, true)).transpose()?;
    let before = call.args.before.as_deref().map(|text| reader.bound(text, false)).transpose()?;
    // The options as the search's lines name them again, but for the page.
    let mut options = Vec::new();
    if any {
        options.push("--any".to_owned());
    }
    if let Some(side) = call.args.from {
        options.push(format!("--from {}", side.as_str()));
    }
    if let Some((when, _)) = &after {
        options.push(format!("--after {when}"));
    }
    if let Some((when, _)) = &before {
        options.push(format!("--before {when}"));
    }
    if oldest {
        options.push("--oldest".to_owned());
    }
    let query = HistoryQuery {
        words: words.clone(),
        any,
        kinds: match call.args.from {
            None => Vec::new(),
            Some(Side::User) => vec![SearchKind::Message],
            Some(Side::Agent) => vec![SearchKind::Answer, SearchKind::Command],
        },
        after: after.map(|(_, bound)| bound),
        before: before.map(|(_, bound)| bound),
        oldest,
    };
    let node = reader.node.clone();
    let agent = shard
        .services()
        .conversations
        .read(&reader.conversation, move |connection| tree::agent_number(connection, &node))
        .await
        .map_err(failed)?
        .flatten()
        .ok_or_else(|| failed("the transcript has no blocks yet"))?;
    let hits = shard
        .services()
        .search
        .find_in(shard.user(), &reader.conversation, agent, query)
        .await
        .map_err(failed)?;
    let command = call.invocation.path.join(" ");
    let pages = hits.len().div_ceil(HITS_PER_PAGE).max(1);
    let page = usize::try_from(call.args.page.unwrap_or(1)).unwrap_or(usize::MAX);
    if page > pages {
        return Err(failed(format!("the search has {pages} pages")));
    }
    let mut rows = Vec::new();
    for &at in hits.iter().skip((page - 1) * HITS_PER_PAGE).take(HITS_PER_PAGE) {
        let (blocks, _) = reader.blocks(&shard, at.saturating_sub(AROUND)..=at + AROUND).await?;
        let first = at.saturating_sub(AROUND);
        let Some(hit) = blocks.get(at - first) else {
            // The transcript no longer holds it, as after an edit the
            // index has not caught up with.
            continue;
        };
        let Some((who, text)) = said(&reader, hit) else {
            continue;
        };
        let row = |index: usize, block: &Block, who: &str, text: String, hit: bool| SearchRow {
            index,
            hit,
            author: who.to_owned(),
            created_at: block.created_at(),
            text,
        };
        let before = blocks[..at - first]
            .iter()
            .enumerate()
            .rev()
            .find_map(|(offset, block)| Some((first + offset, block, said(&reader, block)?)));
        let after = blocks[at - first + 1..]
            .iter()
            .enumerate()
            .find_map(|(offset, block)| Some((at + 1 + offset, block, said(&reader, block)?)));
        if let Some((index, block, (who, text))) = before {
            rows.push(row(index, block, who, excerpt(&text, &words, Cut::Tail), false));
        }
        rows.push(row(at, hit, who, excerpt(&text, &words, Cut::AroundMatch), true));
        if let Some((index, block, (who, text))) = after {
            rows.push(row(index, block, who, excerpt(&text, &words, Cut::Head), false));
        }
    }
    if !hits.is_empty() {
        let mut note = format!("page {page} of {pages}");
        if page < pages {
            // The options first, then `--`, so a word such as `--release` is
            // read as a word.
            let quoted: Vec<String> = words.iter().map(|word| shell_word(word)).collect();
            options.push(format!("--page {}", page + 1));
            note.push_str(&format!(
                "; next: demi conversation history search {} -- {}",
                options.join(" "),
                quoted.join(" ")
            ));
        }
        port.stderr(output::note(&command, &note)).await?;
    }
    if call.invocation.json {
        let text = serde_json::to_string(&rows).map_err(failed)?;
        port.stdout(text + "\n").await?;
        return Ok(0);
    }
    let mut table = Table::new(["index", "hit", "author", "time", "text"]);
    for row in &rows {
        table.row::<Cell>([
            row.index.into(),
            row.hit.into(),
            row.author.as_str().into(),
            row.created_at.into(),
            row.text.as_str().into(),
        ]);
    }
    port.stdout(table.render()).await?;
    Ok(0)
}

/// A row of `demi conversation history search`: a hit, or the message
/// before or after it.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SearchRow {
    /// The block's index.
    index: usize,
    /// Whether the block holds the words; the others are the messages
    /// beside it.
    hit: bool,
    /// `user`, `parent`, `agent` or `command`.
    author: String,
    created_at: Timestamp,
    /// Its text cut around the words, which are marked **like this**.
    text: String,
}

/// Who wrote a block a search finds, and its text as a hit shows it: a
/// command's title with how it stands; none for a block a search does not
/// find.
fn said(reader: &Reader, block: &Block) -> Option<(&'static str, String)> {
    let found = searchable_text(block)?;
    Some(match (found.kind, block) {
        (SearchKind::Message, Block::User(_)) => (reader.author(), found.text),
        (SearchKind::Message, _) => ("user", found.text),
        (SearchKind::Answer, _) => ("agent", found.text),
        (SearchKind::Command, Block::ToolCall(call)) => ("command", command(call)?),
        (SearchKind::Command, _) => return None,
    })
}

/// Whose a block a read with `--from` prints is, as a search with `--from`
/// counts it: a message or a steer is the user's or parent's, a text or a
/// command the agent's; none for any other block.
fn side_of(block: &Block) -> Option<Side> {
    match searchable_text(block)?.kind {
        SearchKind::Message => Some(Side::User),
        SearchKind::Answer | SearchKind::Command => Some(Side::Agent),
    }
}

/// Which part of a text a hit shows.
enum Cut {
    /// Around the first word it holds.
    AroundMatch,
    /// Its start.
    Head,
    /// Its end.
    Tail,
}

/// A text on one line, cut as `cut` says to its share of a hit, with an
/// ellipsis where it is cut and each of `words` marked.
fn excerpt(text: &str, words: &[String], cut: Cut) -> String {
    let characters: Vec<char> = text
        .chars()
        .map(|character| if character.is_whitespace() { ' ' } else { character })
        .collect();
    let found = occurrences(&characters, words);
    let length = characters.len();
    let (start, end) = match cut {
        Cut::AroundMatch => {
            let first = found.first().map_or(0, |&(start, _)| start);
            let start = first.saturating_sub(HIT_CHARS / 4);
            let end = (start + HIT_CHARS).min(length);
            (end.saturating_sub(HIT_CHARS).min(start), end)
        }
        Cut::Head => (0, BESIDE_CHARS.min(length)),
        Cut::Tail => (length.saturating_sub(BESIDE_CHARS), length),
    };
    let mut shown = String::new();
    if start > 0 {
        shown.push('…');
    }
    let mut position = start;
    for &(from, to) in found.iter().filter(|&&(from, to)| to > start && from < end) {
        let (from, to) = (from.max(start), to.min(end));
        shown.extend(&characters[position..from]);
        shown.push_str("**");
        shown.extend(&characters[from..to]);
        shown.push_str("**");
        position = to;
    }
    shown.extend(&characters[position..end]);
    if end < length {
        shown.push('…');
    }
    shown
}

/// A word as a shell reads it back: as it is when it holds no character a
/// shell treats specially, otherwise in single quotes.
fn shell_word(word: &str) -> String {
    let plain = word
        .chars()
        .all(|character| character.is_alphanumeric() || "-_./:@+,%=".contains(character));
    if plain {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// A block index as the commands print it, or with the `#` a quoted one
/// may carry.
fn index(text: &str) -> Option<usize> {
    text.strip_prefix('#').unwrap_or(text).parse().ok()
}

/// The range a read names: `4213..4260`, `4213..` or `..4260`; the end is
/// included, and none means the transcript's end.
fn range(text: &str) -> Option<(usize, Option<usize>)> {
    let (from, to) = text.split_once("..")?;
    let from = if from.is_empty() { 0 } else { index(from)? };
    let to = if to.is_empty() { None } else { Some(index(to)?) };
    (to.is_none_or(|to| from <= to)).then_some((from, to))
}

async fn read(shard: Rc<Shard>, call: Call<ReadArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let reader = Reader::of(&call)?;
    let Some((from, to)) = range(&call.args.range) else {
        return Err(refused(format!(
            "{}: a range is <from>..<to>, <from>.. or ..<to>, block indices such as 4213..4260",
            call.args.range
        )));
    };
    let first_end = to.map_or(from + BATCH - 1, |to| to.min(from + BATCH - 1));
    let (mut blocks, length) = reader.blocks(&shard, from..=first_end).await?;
    if length == 0 {
        return Err(failed("the transcript has no blocks yet"));
    }
    if from >= length {
        return Err(failed(format!(
            "{from} is past the end: the transcript's last block is {}",
            length - 1
        )));
    }
    let end = to.map_or(length - 1, |to| to.min(length - 1));
    let from_side = call.args.from;
    let mut printed = Vec::new();
    let mut used = 0;
    let mut stopped = None;
    let mut cut = None;
    let mut at = from;
    'reading: loop {
        for block in &blocks {
            let kept = from_side.is_none_or(|side| side_of(block) == Some(side));
            if let Some(shown) = item(&reader, at, block).filter(|_| kept) {
                let text = shown.text();
                let cost = text.chars().count() + 1;
                if used + cost > BUDGET {
                    if printed.is_empty() {
                        // One block longer than a read: the first page
                        // `open` prints of it, and where the rest is.
                        let (first, pages) = shown.first_page();
                        printed.push(first);
                        cut = Some((at, pages));
                    } else {
                        stopped = Some(at - 1);
                    }
                    break 'reading;
                }
                used += cost;
                printed.push(text);
            }
            at += 1;
        }
        if blocks.is_empty() || at > end {
            break;
        }
        blocks = reader.blocks(&shard, at..=end.min(at + BATCH - 1)).await?.0;
    }
    if printed.is_empty() {
        printed.push(match from_side {
            None => format!("Nothing to show in {from}..{end}: thinking and context blocks only."),
            Some(Side::User) => format!("No message of the {} in {from}..{end}.", reader.author()),
            Some(Side::Agent) => format!("No text or command of the agent in {from}..{end}."),
        });
    }
    let option = from_side.map_or_else(String::new, |side| format!("--from {} ", side.as_str()));
    let next = |at: usize| match to {
        Some(_) => format!("demi conversation history read {option}{}..{end}", at + 1),
        None => format!("demi conversation history read {option}{}..", at + 1),
    };
    if let Some(at) = stopped {
        printed.push(format!("Stopped at {at} · next: {}", next(at)));
    }
    match cut {
        Some((at, pages)) if pages > 1 => {
            let mut line = format!("Cut {at} · rest: demi conversation history open {at} --page 2");
            if at < end {
                line.push_str(&format!(" · next: {}", next(at)));
            }
            printed.push(line);
        }
        Some((at, _)) if at < end => printed.push(format!("Stopped at {at} · next: {}", next(at))),
        _ => {}
    }
    port.stdout(printed.join("\n") + "\n").await?;
    Ok(0)
}

/// How a read shows a block: the line that names it, and the text under it
/// of a message or an agent's text, which a read shows whole and `open`
/// a page at a time.
struct Shown {
    head: String,
    body: Option<String>,
}

impl Shown {
    fn line(head: String) -> Self {
        Self { head, body: None }
    }

    fn text(&self) -> String {
        match &self.body {
            Some(body) => format!("{}\n{body}", self.head),
            None => self.head.clone(),
        }
    }

    /// The block as far as a read shows one longer than itself, and how
    /// many pages `open` prints of it: the head and the first page of the
    /// text, as `open` prints it; a block of one line, its start.
    fn first_page(&self) -> (String, usize) {
        let Some(body) = &self.body else {
            let cut: String = self.head.chars().take(BUDGET).collect();
            return (format!("{cut}…"), 1);
        };
        let pages = text_pages(body);
        let first: String = body.chars().take(pages[0].end).collect();
        (format!("{}\n{first}…", self.head), pages.len())
    }
}

/// How a read shows the block at `at`: a message or a text whole, under a
/// line that names it; any other block as one line; none for thinking, the
/// blocks the runtime writes for the model alone, and usage.
fn item(reader: &Reader, at: usize, block: &Block) -> Option<Shown> {
    let line = |text: String| Some(Shown::line(text));
    match block {
        Block::User(user) => Some(message(reader, at, reader.author(), user.created_at, &user.content)),
        Block::Steer(steer) => Some(message(reader, at, "user", steer.created_at, &steer.content)),
        Block::Text(text) if !text.text.trim().is_empty() => Some(Shown {
            head: format!("[{at} agent · {}]", reader.when(text.created_at)),
            body: Some(text.text.clone()),
        }),
        Block::AgentMessage(message) => {
            let when = reader.when(message.created_at);
            let from = match &message.message.sender {
                Some(sender) => format!("agent {}", sender.number),
                None => "Demi".to_owned(),
            };
            let kind = match &message.message.event {
                AgentMessageEvent::Completion { .. } => return line(report_line(at, message)),
                AgentMessageEvent::Message {} => "message",
                _ => "notice",
            };
            Some(Shown {
                head: format!("[{at} {kind} from {from} · {when}]"),
                body: Some(message.message.content.clone()),
            })
        }
        Block::ToolCall(call) => line(call_line(at, call)),
        Block::Resume(_) => line(format!("[{at} turn resumed]")),
        Block::Abort(_) => line(format!("[{at} stopped by the user]")),
        Block::Error(error) => line(format!("[{at} error: {}]", error.message)),
        Block::AccountSwitch(switch) => line(format!("[{at} account switched from {} to {}]", switch.from, switch.to)),
        Block::CompactionBoundary(_) => line(format!(
            "[{at} summary of the earlier conversation: demi conversation history open {at}]"
        )),
        Block::Text(_)
        | Block::Thinking(_)
        | Block::RedactedThinking(_)
        | Block::Context(_)
        | Block::Wakeup(_)
        | Block::Response(_)
        | Block::CompactionMarker(_) => None,
    }
}

/// A message under the line that names it, its files each as a line.
fn message(reader: &Reader, at: usize, author: &str, time: Timestamp, content: &[UserContentBlock]) -> Shown {
    let mut lines = Vec::new();
    for part in content {
        match part {
            UserContentBlock::Text { text } => lines.push(text.clone()),
            UserContentBlock::Reference { reference } => lines.push(reference.clone()),
            _ => {}
        }
    }
    for (number, part) in files(content) {
        lines.push(format!("[{at}/a{number} {}]", file_kind(part)));
    }
    Shown {
        head: format!("[{at} {author} · {}]", reader.when(time)),
        body: Some(lines.join("\n")),
    }
}

/// The files of a message, numbered from 1: each attachment, and each
/// image, video or PDF that is not the model's view of the attachment
/// after it, which an upload becomes before its record.
fn files(content: &[UserContentBlock]) -> Vec<(usize, &UserContentBlock)> {
    let mut found = Vec::new();
    for (position, part) in content.iter().enumerate() {
        let view_of_next = matches!(content.get(position + 1), Some(UserContentBlock::Attachment(_)));
        let numbered = match part {
            UserContentBlock::Attachment(_) => true,
            UserContentBlock::Image { .. } | UserContentBlock::Video { .. } | UserContentBlock::Document { .. } => {
                !view_of_next
            }
            UserContentBlock::Text { .. } | UserContentBlock::Reference { .. } => false,
        };
        if numbered {
            found.push((found.len() + 1, part));
        }
    }
    found
}

fn file_kind(part: &UserContentBlock) -> String {
    match part {
        UserContentBlock::Attachment(attachment) => format!("attachment: {}", attachment.name),
        UserContentBlock::Image { .. } => "image".to_owned(),
        UserContentBlock::Video { .. } => "video".to_owned(),
        UserContentBlock::Document { source: DocumentSource::Ref { file_name, .. } } => format!("PDF: {file_name}"),
        UserContentBlock::Text { .. } | UserContentBlock::Reference { .. } => unreachable!("not a file"),
    }
}

/// A call as one line: a command's title, how it stands and how long it
/// ran; any other tool by its name.
fn call_line(at: usize, call: &ToolCallBlock) -> String {
    match command(call) {
        Some(command) => format!("[{at} command: {command}]"),
        None => format!("[{at} {} call]", call.tool_name),
    }
}

/// A command as a line names it: its title, how it stands and how long it
/// ran, such as `Run the tests · exit 0 · 3s`; none for a call of another
/// tool.
fn command(call: &ToolCallBlock) -> Option<String> {
    let title = call.shell_command().map(|command| command.title);
    match (&call.view, title) {
        (Some(ToolView::Shell(view)), title) => {
            let title = title.unwrap_or_else(|| "a command".to_owned());
            let state = match view.status {
                ShellViewStatus::Running => "running".to_owned(),
                ShellViewStatus::Exited => match view.exit_code {
                    Some(code) => format!("exit {code}"),
                    None => "exited".to_owned(),
                },
                ShellViewStatus::Aborted => "stopped".to_owned(),
                ShellViewStatus::Lost => "lost".to_owned(),
            };
            Some(format!("{title} · {state} · {}", duration(view.running_ms)))
        }
        (Some(ToolView::RepeatedShell { .. }), Some(title)) => Some(format!("{title} · not run again")),
        _ => None,
    }
}

/// A subagent's report as one line: who, and how its round ended.
fn report_line(at: usize, message: &demi_shared_types::AgentMessageBlock) -> String {
    let who = message
        .message
        .sender
        .as_ref()
        .map_or_else(|| "a subagent".to_owned(), |sender| format!("agent {} ({})", sender.number, sender.description));
    let AgentMessageEvent::Completion { outcome } = &message.message.event else {
        unreachable!("a report is a completion")
    };
    format!("[{at} report of {who} · {outcome}]")
}

/// What `open` names: a block, or a file of a message.
struct Item {
    at: usize,
    file: Option<usize>,
}

fn open_item(text: &str) -> Option<Item> {
    let (block, file) = match text.split_once('/') {
        Some((block, file)) => (block, Some(file.strip_prefix('a')?.parse().ok()?)),
        None => (text, None),
    };
    Some(Item { at: index(block)?, file })
}

async fn open(shard: Rc<Shard>, call: Call<OpenArgs>, port: RpcPort) -> Result<u8, RpcError> {
    let reader = Reader::of(&call)?;
    let Some(named) = open_item(&call.args.item) else {
        return Err(refused(format!(
            "{}: an item is a block index, such as 4214, or a file of a message, such as 4213/a1",
            call.args.item
        )));
    };
    let page = call.args.page.unwrap_or(1);
    let at = named.at;
    let (mut blocks, length) = reader.blocks(&shard, at..=at).await?;
    let Some(block) = blocks.pop() else {
        return Err(failed(match length {
            0 => "the transcript has no blocks yet".to_owned(),
            length => format!("{at} is past the end: the transcript's last block is {}", length - 1),
        }));
    };
    if let Some(file) = named.file {
        let content = match &block {
            Block::User(user) => &user.content,
            Block::Steer(steer) => &steer.content,
            _ => return Err(failed(format!("{at} is not a message, which has no files"))),
        };
        let Some((_, part)) = files(content).into_iter().find(|(number, _)| *number == file) else {
            return Err(failed(format!("{at} has no file a{file}")));
        };
        let saved = save(&shard, &call, &format!("{at}-a{file}"), part, &port.cancellation()).await?;
        port.stdout(format!("[{at}/a{file} {}] saved to {saved}\n", file_kind(part)))
            .await?;
        return Ok(0);
    }
    let printed = match &block {
        Block::ToolCall(tool) => match (&tool.view, tool.shell_command().map(|command| command.script)) {
            (Some(ToolView::Shell(view)), Some(script)) => {
                let Some(tree) = shard.agent().tree(&root_of(&reader.conversation)) else {
                    return Err(failed("the conversation's agents are not running"));
                };
                // The script and the output are paged together, with room
                // on each page for the line that names the next.
                let (printed, pages) = tree
                    .command_page(&view.command_id, &call_line(at, tool), &script, page, NEXT_CHARS)
                    .await
                    .map_err(failed)?;
                printed + &next_line(at, page, pages)
            }
            (Some(ToolView::RepeatedShell { .. }), Some(script)) => {
                paged(&call_line(at, tool), &format!("$ {script}"), at, page)?
            }
            _ => return Err(failed(format!("{at} is a {} call, which has nothing to open", tool.tool_name))),
        },
        Block::Thinking(thinking) => paged(&format!("[{at} thinking]"), &thinking.text, at, page)?,
        Block::AgentMessage(message) if matches!(message.message.event, AgentMessageEvent::Completion { .. }) => {
            paged(&report_line(at, message), &message.message.content, at, page)?
        }
        Block::CompactionBoundary(boundary) => {
            paged(&format!("[{at} summary of the earlier conversation]"), &boundary.summary, at, page)?
        }
        block => match item(&reader, at, block) {
            Some(Shown { head, body: Some(body) }) => paged(&head, &body, at, page)?,
            _ => {
                return Err(failed(format!(
                    "a read shows all of {at}: demi conversation history read {at}..{at}"
                )));
            }
        },
    };
    port.stdout(printed).await?;
    Ok(0)
}

/// The room a page of `open` leaves for its last line, which names the
/// next page.
const NEXT_CHARS: usize = 100;

/// The room a page of `open` leaves for the line that names its block.
const HEAD_CHARS: usize = 200;

/// The characters of `body` each page of `open` shows, under the line that
/// names the block and above the line that names the next page: the one
/// cut a read's cut block and `open`'s pages both follow, so every page a
/// line names exists. The size is the same for every block, since the head
/// of one block changes, as its time gains its year.
fn text_pages(body: &str) -> Vec<Range<usize>> {
    let size = BUDGET - HEAD_CHARS - NEXT_CHARS;
    let length = body.chars().count();
    (0..length.div_ceil(size).max(1))
        .map(|page| page * size..((page + 1) * size).min(length))
        .collect()
}

/// Page `page` of `text` under `head`, with the command for the next page
/// when one follows.
fn paged(head: &str, text: &str, at: usize, page: u64) -> Result<String, RpcError> {
    let pages = text_pages(text);
    let Some(range) = usize::try_from(page).ok().and_then(|page| pages.get(page - 1)) else {
        return Err(failed(format!("{at} has {} pages", pages.len())));
    };
    let body: String = text.chars().skip(range.start).take(range.len()).collect();
    let count = u64::try_from(pages.len()).unwrap_or(u64::MAX);
    Ok(format!("{head}\n{body}\n{}", next_line(at, page, count)))
}

/// The line under page `page` of `pages` of what `open` prints of `at`,
/// which names the next page; nothing under the last.
fn next_line(at: usize, page: u64, pages: u64) -> String {
    if page >= pages {
        return String::new();
    }
    format!(
        "Page {page} of {pages} · next: demi conversation history open {at} --page {}\n",
        page + 1
    )
}

/// Saves a message's file in the calling agent's scratch directory on the
/// Host the calling job runs on, inside the one the Host's runner names
/// (`crates/runner-jobs/src/scratch.rs`), as `<name>-<file name>` or the first
/// free name after it, and answers the path. A call stopped before the file
/// is written, `cancel`, writes nothing.
async fn save<A>(
    shard: &Shard,
    call: &Call<A>,
    name: &str,
    part: &UserContentBlock,
    cancel: &CancellationToken,
) -> Result<String, RpcError> {
    let (blob, file_name) = match part {
        UserContentBlock::Attachment(attachment) => (&attachment.sha256, attachment.name.clone()),
        UserContentBlock::Image { source: MediaSource::Ref { r#ref, media_type, .. } }
        | UserContentBlock::Video { source: MediaSource::Ref { r#ref, media_type, .. } } => {
            (r#ref, format!("file.{}", extension(media_type)))
        }
        UserContentBlock::Document { source: DocumentSource::Ref { r#ref, file_name, .. } } => (r#ref, file_name.clone()),
        UserContentBlock::Image { source: MediaSource::Url { url } }
        | UserContentBlock::Video { source: MediaSource::Url { url } } => {
            return Err(failed(format!("the file is not stored; the model read it from {url}")));
        }
        UserContentBlock::Text { .. } | UserContentBlock::Reference { .. } => unreachable!("not a file"),
    };
    let invocation = &call.invocation;
    let hosts: &dyn HostShard = shard;
    let bytes = hosts
        .blobs()
        .get(blob)
        .await
        .map_err(failed)?
        .ok_or_else(|| failed("the file's bytes are no longer stored"))?;
    let conversation = conversation_of(invocation)?;
    let device = DeviceId::try_from(invocation.host.as_str())
        .map_err(|_| failed(format!("{}: names no device", invocation.host)))?;
    let file = format!("{name}-{file_name}");
    hosts
        .with_host(&conversation, Some(&device), cancel, async |host| {
            let identity = host.host.identity();
            if identity.scratch_root.is_empty() {
                return Err("the Host has not named its scratch directory".to_owned());
            }
            // The directory the runner makes for the calling agent, named
            // as the job's command context names it, which its job made.
            let mut scratch = identity.paths.join(&identity.scratch_root, &invocation.context.conversation);
            if let Some(folder) = invocation.context.caller.scratch_folder() {
                scratch = identity.paths.join(&scratch, &folder);
            }
            let options = WriteOptions { exists: WhenExists::Rename };
            let path = identity.paths.join(&scratch, &file);
            // The admission ends with a stopped call; one stopped since
            // writes nothing all the same.
            if cancel.is_cancelled() {
                return Err("the command was stopped".to_owned());
            }
            let written = host
                .host
                .fs()
                .write_file(&path, FileContents::Bytes(bytes), options)
                .await
                .map_err(|error| error.message)?;
            Ok(identity.paths.join(&scratch, &written))
        })
        .await
        .map_err(failed)?
        .map_err(failed)
}

/// The usual extension of an image's or a video's media type.
fn extension(media_type: &str) -> &str {
    match media_type {
        "image/jpeg" => "jpg",
        "image/svg+xml" => "svg",
        "video/quicktime" => "mov",
        other => other.split('/').nth(1).unwrap_or("bin"),
    }
}
