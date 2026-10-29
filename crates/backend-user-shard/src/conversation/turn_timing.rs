//! Where the time of a turn went, from the times its blocks keep: when each
//! part of it started and ended.
//!
//! ```text
//! message   preparing  waiting  thinking  calls  tools     preparing  waiting  text
//! |---------|----------|--------|---------|------|---------|----------|--------|----|
//!           sent_at    first_output_at    writing_started_at          sent_at
//! ```
//!
//! A turn is the blocks of one turn id, from its first block's start to the
//! latest time any of them keeps. Each part is a set of spans: the model
//! waited for (a request's first output), thinking, text and calls being
//! written, calls running once their response ended, a compaction, and the
//! time before a request left, Demi's own after what ended last, or a failed
//! request and the wait after it. Time two parts share counts once, for the
//! one that ranks first in [`Part`]; what no part covers is other, so the
//! parts add up to the turn.
//!
//! What happened outside the turn is not its time: a compaction's boundary,
//! written where the kept history begins, a pass that ran while the
//! conversation was idle, and a failure outside any turn.

use demi_shared_types::{Block, Timestamp};
use schemars::JsonSchema;
use serde::Serialize;

/// A part of a turn, in the order a time two of them share counts for.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Part {
    Tools,
    Compacting,
    Thinking,
    Text,
    Calls,
    Waiting,
    Retrying,
    Preparing,
}

const PARTS: usize = 8;

/// Where the time of one turn went, in milliseconds.
#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub(super) struct TurnTiming {
    /// The index of the turn's first block.
    pub index: usize,
    pub started_at: Timestamp,
    /// From the turn's first block's start to the latest time its blocks
    /// keep.
    pub ms: u64,
    /// The provider requests that ended with a response.
    pub requests: u64,
    /// Demi's own time before a request left, after what ended last.
    pub preparing_ms: u64,
    /// From a request leaving to its first output.
    pub waiting_ms: u64,
    pub thinking_ms: u64,
    /// The model writing its answer's text.
    pub text_ms: u64,
    /// The model writing tool calls.
    pub calls_ms: u64,
    /// The calls running.
    pub tools_ms: u64,
    pub compacting_ms: u64,
    /// A failed request and the wait before the next.
    pub retrying_ms: u64,
    /// The time no part covers.
    pub other_ms: u64,
    /// Whether a block of the turn was written before the transcript kept
    /// its times, whose time then counts as other.
    pub untimed: bool,
}

/// A turn as its blocks come.
pub(super) struct Turn {
    index: usize,
    started_at: Timestamp,
    start: i64,
    end: i64,
    /// The latest time anything of the turn ended, and whether a failure
    /// set it.
    settled: i64,
    settled_by_failure: bool,
    /// What was settled when the first block of the request that streams
    /// now came: the request left after it. A request's blocks come before
    /// its response, which says when it left.
    request_from: Option<(i64, bool)>,
    /// The calls of the request that streams now, by when each was written
    /// and returned: they run once its response ended.
    calls: Vec<(i64, Option<i64>)>,
    requests: u64,
    spans: Vec<(Part, i64, i64)>,
    untimed: bool,
}

impl Turn {
    fn new(index: usize, started_at: Timestamp) -> Self {
        let start = started_at.as_millisecond();
        Self {
            index,
            started_at,
            start,
            end: start,
            settled: start,
            settled_by_failure: false,
            request_from: None,
            calls: Vec::new(),
            requests: 0,
            spans: Vec::new(),
            untimed: false,
        }
    }

    fn span(&mut self, part: Part, from: Timestamp, to: Timestamp) {
        let (from, to) = (from.as_millisecond(), to.as_millisecond());
        if to > from {
            self.spans.push((part, from, to));
        }
        self.ended(to, false);
    }

    fn ended(&mut self, at: i64, failure: bool) {
        self.end = self.end.max(at);
        if at >= self.settled {
            self.settled = at;
            self.settled_by_failure = failure;
        }
    }

    /// What was settled when the request that streams now began to stream,
    /// or now when none of its blocks came yet; the request is over.
    fn take_request_from(&mut self) -> (i64, bool) {
        self.request_from
            .take()
            .unwrap_or((self.settled, self.settled_by_failure))
    }

    /// The calls of the request that streams now ran from `ran`, or from
    /// when each was written when no response ended the request.
    fn calls_ran(&mut self, ran: Option<i64>) {
        for (written, returned) in std::mem::take(&mut self.calls) {
            let start = ran.map_or(written, |ran| ran.max(written));
            if let Some(returned) = returned
                && returned > start
            {
                self.spans.push((Part::Tools, start, returned));
            }
        }
    }

    fn add(&mut self, block: &Block) {
        let created = block.created_at();
        let outside = match block {
            Block::CompactionBoundary(_) => true,
            Block::CompactionMarker(marker) => marker.idle,
            Block::Error(error) => error.outside_turn,
            _ => false,
        };
        if outside {
            return;
        }
        let streamed = matches!(
            block,
            Block::Thinking(_) | Block::RedactedThinking(_) | Block::Text(_) | Block::ToolCall(_)
        );
        if streamed && self.request_from.is_none() {
            self.request_from = Some((self.settled, self.settled_by_failure));
        }
        match block {
            Block::Thinking(thinking) => match thinking.ended_at {
                Some(end) => self.span(Part::Thinking, created, end),
                None => self.untimed = true,
            },
            Block::Text(text) => match text.ended_at {
                Some(end) => self.span(Part::Text, created, end),
                None => self.untimed = true,
            },
            Block::ToolCall(call) => {
                if let Some(start) = call.writing_started_at {
                    self.span(Part::Calls, start, created);
                }
                let returned = call.ended_at.map(Timestamp::as_millisecond);
                self.calls.push((created.as_millisecond(), returned));
                if let Some(returned) = returned {
                    self.ended(returned, false);
                }
            }
            Block::Response(response) => {
                self.requests += 1;
                self.calls_ran(Some(created.as_millisecond()));
                let (from, after_failure) = self.take_request_from();
                let Some(sent) = response.sent_at else {
                    self.untimed = true;
                    self.ended(created.as_millisecond(), false);
                    return;
                };
                // Attempts that failed and were tried again left no block:
                // the response says when the first of them left.
                let first_sent = response.first_sent_at.unwrap_or(sent).as_millisecond();
                let before = if after_failure { Part::Retrying } else { Part::Preparing };
                if first_sent > from {
                    self.spans.push((before, from, first_sent));
                }
                if sent.as_millisecond() > first_sent {
                    self.spans.push((Part::Retrying, first_sent, sent.as_millisecond()));
                }
                if let Some(first) = response.first_output_at {
                    self.span(Part::Waiting, sent, first);
                }
            }
            Block::Error(_) => {
                // A failed request ran from what ended before it.
                let at = created.as_millisecond();
                self.calls_ran(None);
                let (from, _) = self.take_request_from();
                if at > from {
                    self.spans.push((Part::Retrying, from, at));
                }
                self.ended(at, true);
                return;
            }
            Block::CompactionMarker(marker) => match marker.started_at {
                Some(start) => self.span(Part::Compacting, start, created),
                None => self.untimed = true,
            },
            _ => {}
        }
        self.ended(created.as_millisecond(), false);
    }

    /// Where the turn's time went, each instant counted for one part.
    pub(super) fn timing(mut self) -> TurnTiming {
        // Calls no response ended, as a stopped turn's, ran as written.
        self.calls_ran(None);
        let mut cuts: Vec<i64> = self.spans.iter().flat_map(|&(_, from, to)| [from, to]).collect();
        cuts.extend([self.start, self.end]);
        cuts.retain(|&at| (self.start..=self.end).contains(&at));
        cuts.sort_unstable();
        cuts.dedup();
        let mut parts = [0_u64; PARTS];
        for pair in cuts.windows(2) {
            let (from, to) = (pair[0], pair[1]);
            let part = self
                .spans
                .iter()
                .filter(|&&(_, start, end)| start <= from && to <= end)
                .map(|&(part, _, _)| part)
                .min();
            if let Some(part) = part {
                parts[part as usize] += u64::try_from(to - from).unwrap_or(0);
            }
        }
        let ms = u64::try_from(self.end - self.start).unwrap_or(0);
        let covered: u64 = parts.iter().sum();
        TurnTiming {
            index: self.index,
            started_at: self.started_at,
            ms,
            requests: self.requests,
            preparing_ms: parts[Part::Preparing as usize],
            waiting_ms: parts[Part::Waiting as usize],
            thinking_ms: parts[Part::Thinking as usize],
            text_ms: parts[Part::Text as usize],
            calls_ms: parts[Part::Calls as usize],
            tools_ms: parts[Part::Tools as usize],
            compacting_ms: parts[Part::Compacting as usize],
            retrying_ms: parts[Part::Retrying as usize],
            other_ms: ms.saturating_sub(covered),
            untimed: self.untimed,
        }
    }
}

/// The turn id `block` names, for a block that names one.
pub(super) fn turn_of(block: &Block) -> Option<&str> {
    match block {
        Block::User(block) => Some(block.turn_id.as_str()),
        Block::Context(block) => Some(block.turn_id.as_str()),
        Block::Wakeup(block) => Some(block.turn_id.as_str()),
        Block::Steer(block) => Some(block.turn_id.as_str()),
        Block::AgentMessage(block) => Some(block.turn_id.as_str()),
        Block::Resume(block) => Some(block.turn_id.as_str()),
        _ => None,
    }
}

/// The turns of a transcript that start from some block on, as its blocks
/// come in order.
pub(super) struct Turns {
    done: Vec<Turn>,
    open: Option<Turn>,
    /// The turn the latest block that names one named.
    named: Option<String>,
}

impl Turns {
    /// Turns from a block on, after one of the turn `before` names, the
    /// latest any block before it named, whose blocks are not taken.
    pub(super) fn after(before: Option<String>) -> Self {
        Self {
            done: Vec::new(),
            open: None,
            named: before,
        }
    }

    /// Whether `block` starts a turn: it names another than the blocks
    /// before it.
    pub(super) fn starts(&self, block: &Block) -> bool {
        turn_of(block).is_some_and(|turn| self.named.as_deref() != Some(turn))
    }

    /// Whether a turn is open, whose blocks the next block may continue.
    pub(super) fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Takes the block at `index`: it starts a turn, joins the one open,
    /// or belongs to a turn that started before.
    pub(super) fn add(&mut self, index: usize, block: &Block) {
        if self.starts(block) {
            let turn = Turn::new(index, block.created_at());
            self.done.extend(self.open.replace(turn));
        }
        if let Some(turn) = turn_of(block) {
            self.named = Some(turn.to_owned());
        }
        if let Some(open) = &mut self.open {
            open.add(block);
        }
    }

    /// How many turns started so far.
    pub(super) fn started(&self) -> usize {
        self.done.len() + usize::from(self.open.is_some())
    }

    /// The turns, the open one last.
    pub(super) fn into_turns(self) -> Vec<Turn> {
        self.done.into_iter().chain(self.open).collect()
    }
}
