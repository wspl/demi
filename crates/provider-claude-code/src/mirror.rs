//! Which block each entry belongs to that a process mirrors of its session
//! (`claude-code.md` § The session a process resumes, `--session-mirror`):
//! the user's message to its block, the model's reasoning, text and tool
//! calls to theirs, a call's result to the call's; an entry that stands for
//! no block, such as a `date` attachment, to the next block. The CLI prints
//! a batch of entries when it writes them, which can be after the run that
//! made them ended, so a block of a run is named by its position among the
//! run's blocks while the run lasts, and by its items in later requests.

use std::collections::{HashMap, VecDeque};

use demi_provider_common::{EntriesOf, InferenceItem, InferenceRequest, ProviderEvent};
use serde_json::Value;

use crate::session;

/// Where a block is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Spot {
    /// The block of an item of the requests from now on.
    Item(usize),
    /// The block the run `run` wrote at `index`.
    Output { run: u64, index: usize },
}

/// The last block a run's events wrote, as the agent writes them
/// ([`EntriesOf::Output`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Last {
    /// Reasoning, which more reasoning goes on until it is signed.
    Thinking { signed: bool },
    /// Text, which more text goes on while nothing else came after it.
    Text { open: bool },
    Other,
}

/// Where an entry's block is, as far as it is known now.
enum Placing {
    Block(Spot),
    /// It stands for no block, and belongs to the next one.
    Next,
    /// Its block is not known yet: the line that names it is still to come.
    Later,
}

/// One process's mirror.
#[derive(Default)]
pub(crate) struct Mirror {
    /// The run being read, counted from 1, and how many items its request
    /// carried.
    run: u64,
    base: usize,
    /// How many blocks the run's events wrote, and the last one.
    written: usize,
    last: Option<Last>,
    /// A thinking start's block, which the next event opens.
    starting: bool,
    /// The run's latest block of each kind.
    thinking: Option<usize>,
    redacted: Option<usize>,
    text: Option<usize>,
    /// The block of each content block the CLI printed whole, by its
    /// message's id and the block; none for one that wrote no block.
    contents: Vec<(Option<String>, Value, Option<Spot>)>,
    /// The block of each tool use, by its id.
    tools: HashMap<String, Spot>,
    /// The items of the user messages written to the process, oldest
    /// first, whose entries the CLI has not printed yet.
    given: VecDeque<usize>,
    /// Entries that belong to the next block.
    next: Vec<Value>,
    /// Entries whose block is not known yet, in order.
    waiting: VecDeque<Value>,
}

impl Mirror {
    /// A run of the process begins with `request`: the previous run's
    /// blocks are now items of it, the request's blocks after those the
    /// previous request carried, and entries still waiting for a line of
    /// that run belong to the next block.
    pub(crate) fn begin(&mut self, request: &InferenceRequest) -> Vec<ProviderEvent> {
        let previous = self.run;
        let base = self.base;
        let carried = |index: usize| {
            request
                .blocks
                .iter()
                .filter(|block| block.items.start >= base)
                .nth(index)
                .map(|block| Spot::Item(block.items.start))
        };
        let moved = |spot: Spot| match spot {
            Spot::Output { run, index } if run == previous => carried(index),
            spot => Some(spot),
        };
        for (_, _, spot) in &mut self.contents {
            *spot = spot.and_then(moved);
        }
        self.tools = std::mem::take(&mut self.tools)
            .into_iter()
            .filter_map(|(id, spot)| Some((id, moved(spot)?)))
            .collect();
        self.run += 1;
        self.base = request.items.len();
        self.written = 0;
        self.last = None;
        self.starting = false;
        self.thinking = None;
        self.redacted = None;
        self.text = None;
        self.place(request, true)
    }

    /// Notes an event the run yields: the block it writes, as the agent
    /// writes it.
    pub(crate) fn observe(&mut self, event: &ProviderEvent) {
        match event {
            ProviderEvent::Entries { .. } => return,
            ProviderEvent::ThinkingStart => {
                self.starting = true;
                return;
            }
            _ => {}
        }
        if std::mem::take(&mut self.starting) {
            self.thinking = Some(self.write(Last::Thinking { signed: false }));
        }
        // Anything but more text, or a signature, completes open text.
        if !matches!(
            event,
            ProviderEvent::TextDelta(_) | ProviderEvent::ThinkingSignature(_)
        ) && let Some(Last::Text { open }) = &mut self.last
        {
            *open = false;
        }
        match event {
            ProviderEvent::ThinkingDelta(_)
                if self.last != Some(Last::Thinking { signed: false }) =>
            {
                self.thinking = Some(self.write(Last::Thinking { signed: false }));
            }
            ProviderEvent::ThinkingSignature(_) => {
                if let Some(Last::Thinking { signed }) = &mut self.last {
                    *signed = true;
                }
            }
            ProviderEvent::RedactedThinking(_) => self.redacted = Some(self.write(Last::Other)),
            ProviderEvent::TextDelta(_) if self.last != Some(Last::Text { open: true }) => {
                self.text = Some(self.write(Last::Text { open: true }));
            }
            ProviderEvent::ToolCall(call) => {
                let index = self.write(Last::Other);
                let spot = Spot::Output {
                    run: self.run,
                    index,
                };
                self.tools.insert(call.tool_use_id.clone(), spot);
            }
            _ => {}
        }
    }

    /// A block the run's events write: its position.
    fn write(&mut self, last: Last) -> usize {
        self.last = Some(last);
        self.written += 1;
        self.written - 1
    }

    /// Notes a content block the CLI printed whole, of the message `id`,
    /// after the events it gave: the block of the run it went to, the
    /// latest of its kind. A tool use is known by its id, and a block that
    /// gave no event wrote none.
    pub(crate) fn printed(&mut self, id: Option<String>, block: Value, wrote: bool) {
        let latest = match block["type"].as_str() {
            Some("thinking") => self.thinking,
            Some("redacted_thinking") => self.redacted,
            Some("text") => self.text,
            _ => return,
        };
        let spot = latest.filter(|_| wrote).map(|index| Spot::Output {
            run: self.run,
            index,
        });
        self.contents.push((id, block, spot));
    }

    /// Notes the user messages at `items` of the request, written to the
    /// process in order.
    pub(crate) fn gave(&mut self, items: impl IntoIterator<Item = usize>) {
        self.given.extend(items);
    }

    /// The events of a batch of entries the CLI printed: each entry that
    /// names its block, with the entries before it that belong to the next
    /// block. Entries the CLI keeps outside its chain, which have no id,
    /// belong to no block.
    pub(crate) fn mirrored(
        &mut self,
        entries: Vec<Value>,
        request: &InferenceRequest,
    ) -> Vec<ProviderEvent> {
        self.waiting
            .extend(entries.into_iter().filter(|entry| entry["uuid"].is_string()));
        self.place(request, false)
    }

    /// The events of the entries whose block is now known, in order; with
    /// `settle`, an entry whose block is still unknown belongs to the next.
    pub(crate) fn place(&mut self, request: &InferenceRequest, settle: bool) -> Vec<ProviderEvent> {
        let mut events: Vec<ProviderEvent> = Vec::new();
        while let Some(entry) = self.waiting.front() {
            let placing = placing(
                entry,
                request,
                &mut self.tools,
                &mut self.contents,
                &mut self.given,
            );
            let spot = match placing {
                Placing::Block(spot) => spot,
                Placing::Later if !settle => break,
                Placing::Next | Placing::Later => {
                    let entry = self.waiting.pop_front().expect("an entry was looked at");
                    self.next.push(entry);
                    continue;
                }
            };
            let entry = self.waiting.pop_front().expect("an entry was looked at");
            let mut entries = std::mem::take(&mut self.next);
            entries.push(entry);
            let of = match spot {
                Spot::Item(index) => EntriesOf::Item(index),
                Spot::Output { run, index } if run == self.run => EntriesOf::Output(index),
                // A block of a run whose request is gone.
                Spot::Output { .. } => continue,
            };
            let entries = match of {
                EntriesOf::Item(index) => kept(entries, request, index),
                EntriesOf::Output(_) => entries,
            };
            match events.last_mut() {
                Some(ProviderEvent::Entries { of: last, entries: held }) if *last == of => {
                    held.extend(entries);
                }
                _ => events.push(ProviderEvent::Entries { of, entries }),
            }
        }
        events
    }
}

/// Where `entry`'s block is, by the tool uses and content blocks the run
/// printed, and the user messages `given` to the process whose entries are
/// still to come.
fn placing(
    entry: &Value,
    request: &InferenceRequest,
    tools: &mut HashMap<String, Spot>,
    contents: &mut Vec<(Option<String>, Value, Option<Spot>)>,
    given: &mut VecDeque<usize>,
) -> Placing {
    let message = &entry["message"];
    let first = &message["content"][0];
    match entry["type"].as_str() {
        // A content block's one entry is placed once, so what printed it
        // is let go.
        Some("assistant") if first["type"] == "tool_use" => first["id"]
            .as_str()
            .and_then(|id| tools.remove(id))
            .map_or(Placing::Later, Placing::Block),
        Some("assistant") => {
            let id = message["id"].as_str();
            let printed = contents
                .iter()
                .position(|(printed, block, _)| printed.as_deref() == id && block == first);
            match printed.map(|index| contents.remove(index).2) {
                Some(Some(spot)) => Placing::Block(spot),
                Some(None) => Placing::Next,
                None => Placing::Later,
            }
        }
        Some("user") => {
            let results = message["content"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|block| block["type"] == "tool_result")
                .find_map(|block| block["tool_use_id"].as_str());
            if let Some(id) = results {
                return result_item(request, id).map_or(Placing::Next, |index| {
                    Placing::Block(Spot::Item(index))
                });
            }
            // The CLI's own words, such as asking the model to go on.
            if entry["isMeta"] == true {
                return Placing::Next;
            }
            given
                .pop_front()
                .map_or(Placing::Next, |index| Placing::Block(Spot::Item(index)))
        }
        // A message given to the process while it worked, which the CLI
        // folds into the running turn.
        Some("attachment") if entry["attachment"]["type"] == "queued_command" => given
            .pop_front()
            .map_or(Placing::Next, |index| Placing::Block(Spot::Item(index))),
        _ => Placing::Next,
    }
}

/// The item of the latest result of `tool_use_id` in `request`.
fn result_item(request: &InferenceRequest, tool_use_id: &str) -> Option<usize> {
    request.items.iter().rposition(|item| {
        matches!(item, InferenceItem::ToolResult { tool_use_id: id, .. } if id == tool_use_id)
    })
}

/// `entries` as the block of the request's item `index` keeps them: a
/// medium the block holds as a reference to it.
fn kept(entries: Vec<Value>, request: &InferenceRequest, index: usize) -> Vec<Value> {
    let Some(block) = request
        .blocks
        .iter()
        .find(|block| block.items.contains(&index))
    else {
        return entries;
    };
    let media = session::media(&request.items[block.items.clone()]);
    session::referred(entries, &media)
}
