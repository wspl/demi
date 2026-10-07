//! Finding a conversation (`product.md` § Finding a conversation,
//! `web-api.md` § Search, `storage.md` § Search index): the user's indexer,
//! which keeps the user's search index in step with the conversations, and
//! the search the edge answers from that index alone.
//!
//! The indexer follows the user's changes as a page does: each change of a
//! conversation's summary, among them every save of its root's checkpoint
//! and every title change, marks the conversation on the indexer's watch.
//! A marked conversation is indexed at once, and a mark within the interval
//! after that waits for its end, so a streaming answer is indexed at most
//! once per interval and its last save, which ends the turn, once more
//! after it. Indexing reads the conversation's record and the saved version
//! of its root's transcript, and writes only when either differs from what
//! the index holds, so a mark that changed neither, such as a read
//! acknowledgement, costs two reads.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::conversation_index::ConversationRecord;
use demi_backend_database::search::{IndexedConversation, IndexedMessage};
use demi_backend_database::tree;
use demi_backend_page_sync::{Part, Registration};
use demi_shared_gates::KeyedSerialGate;
use demi_shared_types::Block;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use demi_web_api_protocol::search::{
    LINE_MAX, RESULTS_MAX, SearchMatch, SearchResult, SearchText,
};

use super::titles::message_text;
use crate::services::Services;
use crate::shard::{Shard, Shards};

/// The user's indexer: the watch of the user's changes, one indexing at a
/// time per conversation, and the conversations marked since their last
/// indexing began.
pub(crate) struct SearchIndexer {
    watch: Registration,
    turns: KeyedSerialGate<ConversationId>,
    /// Each conversation being indexed or waiting out its interval, with
    /// whether it was marked again meanwhile.
    marked: RefCell<HashMap<ConversationId, Rc<Cell<bool>>>>,
}

impl SearchIndexer {
    pub(crate) fn new(services: &Services, user: &UserId) -> Self {
        Self {
            watch: services.sync.watch(user),
            turns: KeyedSerialGate::new(),
            marked: RefCell::default(),
        }
    }
}

impl Shard {
    /// Indexes each conversation the user's changes mark, until the shard
    /// closes.
    pub(crate) async fn follow_for_search(self: Rc<Self>) {
        loop {
            tokio::select! {
                () = self.search_indexer().watch.marked() => {}
                () = self.closed() => return,
            }
            for part in self.search_indexer().watch.take().parts {
                if let Part::Conversation(id) = part {
                    self.mark_for_search(id);
                }
            }
        }
    }

    /// Indexes the conversation now, unless an indexing of it began within
    /// the interval: then once the interval has passed.
    fn mark_for_search(&self, id: ConversationId) {
        let indexer = self.search_indexer();
        if let Some(again) = indexer.marked.borrow().get(&id) {
            again.set(true);
            return;
        }
        let again = Rc::new(Cell::new(false));
        indexer.marked.borrow_mut().insert(id.clone(), again.clone());
        let shard = self.this();
        let interval = self.services().conversation_tuning.search_interval;
        self.tasks().spawn_local(async move {
            loop {
                let record = shard.services().control.conversation(id.clone()).await;
                let indexed = match record {
                    Ok(record) => shard.index_for_search(&id, record).await,
                    Err(error) => Err(error),
                };
                if let Err(error) = indexed {
                    // The index keeps what it had; the conversation's next
                    // change, or the next start, indexes it again.
                    tracing::warn!(
                        conversation = %id,
                        error = &error as &dyn std::error::Error,
                        "a conversation was not indexed for search"
                    );
                }
                tokio::select! {
                    () = tokio::time::sleep(interval) => {}
                    () = shard.closed() => break,
                }
                if !again.replace(false) {
                    break;
                }
            }
            shard.search_indexer().marked.borrow_mut().remove(&id);
        });
    }

    /// Brings the index's rows of the conversation `id` in step with
    /// `record`, its record now: removed when it is gone or not the user's,
    /// otherwise indexed again when its title or its root's saved transcript
    /// differs from what the index holds.
    async fn index_for_search(
        &self,
        id: &ConversationId,
        record: Option<ConversationRecord>,
    ) -> Result<(), StorageError> {
        let _turn = self.search_indexer().turns.acquire(id.clone()).await;
        let services = self.services();
        let user = self.user();
        let Some(record) = record.filter(|record| record.owner == *user) else {
            return services.search.remove_conversation(user, id).await;
        };
        let indexed = services.search.indexed(user, &record.id).await?;
        let version = services
            .conversations
            .read(&record.id, tree::root_version)
            .await?
            .unwrap_or_default();
        if indexed.is_some_and(|indexed| indexed.version == version && indexed.title == record.title)
        {
            return Ok(());
        }
        let (version, blocks) = services
            .conversations
            .read(&record.id, tree::root_transcript)
            .await?
            .unwrap_or_default();
        let conversation = IndexedConversation {
            version,
            title: record.title,
            messages: searchable(&blocks),
        };
        services.search.index(user, &record.id, conversation).await
    }

    /// Brings the whole index in step at start, in the background: each of
    /// the user's conversations, the most recently active first, and then
    /// the rows of any conversation that is gone. An index built again is
    /// empty, so this fills it.
    async fn index_at_start(self: Rc<Self>) {
        if let Err(error) = self.index_everything().await {
            // The conversations not reached are indexed by their next
            // change, or at the next start.
            tracing::warn!(
                user = %self.user(),
                error = &error as &dyn std::error::Error,
                "the search index was not brought in step at start"
            );
        }
    }

    async fn index_everything(&self) -> Result<(), StorageError> {
        let services = self.services();
        let user = self.user();
        let mut records = services.control.conversations(user.clone(), false).await?;
        records.extend(services.control.conversations(user.clone(), true).await?);
        records.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        // The index holds each conversation under its record's spelling.
        let present: HashSet<ConversationId> =
            records.iter().map(|record| record.id.clone()).collect();
        for record in records {
            if self.is_closing() {
                return Ok(());
            }
            let id = record.id.clone();
            self.index_for_search(&id, Some(record)).await?;
        }
        for id in services.search.conversations(user).await? {
            if !present.contains(&id) {
                self.index_for_search(&id, None).await?;
            }
        }
        Ok(())
    }
}

/// Brings each user's search index in step with the conversations at start
/// (`storage.md` § Search index): every user who has a conversation has
/// their shard index it in the background, while the backend serves.
/// Answers once each shard has its task.
pub async fn index_at_start(
    control: &ControlService,
    shards: &Shards,
) -> Result<(), StorageError> {
    for owner in control.conversation_owners().await? {
        let handed = shards.of(&owner).adopt(Shard::index_at_start).await;
        if handed.is_err() {
            // The backend is shutting down; the next start indexes what is
            // left.
            return Ok(());
        }
    }
    Ok(())
}

/// What a search finds of a transcript: the text of each of the root's
/// `user` messages and steers, the user's messages both, and of its answers. Thinking, tool calls and their
/// results, commands' output, files and the blocks the runtime writes are
/// left out, and so is a message without text.
fn searchable(blocks: &[Block]) -> Vec<IndexedMessage> {
    blocks
        .iter()
        .enumerate()
        .filter_map(|(position, block)| {
            let text = match block {
                Block::User(_) | Block::Steer(_) => message_text(block)?,
                Block::Text(answer) if !answer.text.trim().is_empty() => answer.text.clone(),
                _ => return None,
            };
            Some(IndexedMessage {
                block: block.id().clone(),
                position,
                text,
            })
        })
        .collect()
}

/// Answers `query` for `user` from the user's index alone: no
/// conversation's database is opened and no Host woken. The conversations'
/// records give their current titles, archive state and activity, and a
/// conversation whose record is gone is left out.
pub async fn search(
    services: &Services,
    user: &UserId,
    query: &SearchText,
) -> Result<Vec<SearchResult>, StorageError> {
    let words = query.words();
    let found = services.search.find(user, words.clone()).await?;
    // The index holds each conversation under its record's spelling.
    let mut records: HashMap<ConversationId, ConversationRecord> = HashMap::new();
    for archived in [false, true] {
        for record in services.control.conversations(user.clone(), archived).await? {
            records.insert(record.id.clone(), record);
        }
    }
    let mut hits: Vec<_> = found
        .into_iter()
        .filter_map(|found| {
            let record = records.remove(&found.conversation)?;
            Some((found, record))
        })
        .collect();
    hits.sort_by(|(a, a_record), (b, b_record)| {
        b.title
            .cmp(&a.title)
            .then_with(|| b_record.updated_at.cmp(&a_record.updated_at))
            .then_with(|| a_record.id.cmp(&b_record.id))
    });
    hits.truncate(RESULTS_MAX);
    let rows = hits
        .iter()
        .filter_map(|(found, _)| found.message.as_ref().map(|message| message.text))
        .collect();
    let texts = services.search.texts(user, rows).await?;
    Ok(hits
        .into_iter()
        .map(|(found, record)| {
            let r#match = found.message.and_then(|message| {
                let text = texts.get(&message.text)?;
                let (text, ranges) = match_line(text, &words);
                Some(SearchMatch {
                    block_id: message.block,
                    text,
                    ranges,
                })
            });
            let title_ranges = if found.title {
                title_ranges(&record.title, &words)
            } else {
                Vec::new()
            };
            SearchResult {
                conversation_id: record.id,
                title_ranges,
                title: record.title,
                archived: record.archived,
                last_active_at: record.updated_at,
                r#match,
            }
        })
        .collect())
}

/// How many characters of a long line come before its first match.
const LEAD: usize = 40;

/// The line of `text` around the first place a word matches, at most
/// `LINE_MAX` characters, with an ellipsis where it is cut, and the places
/// of every word in it as UTF-16 offsets, which the page marks.
///
/// FTS5's `snippet` and `highlight` would mark only the words of its MATCH,
/// which leaves out the words of one or two characters the index finds by
/// a scan, and they cut by tokens, which the trigram tokenizer makes of
/// every character; neither gives UTF-16 offsets. So the line is cut and
/// the words found here, on the text the index matched, case ignored as the
/// index ignores it.
fn match_line(text: &str, words: &[String]) -> (String, Vec<[u32; 2]>) {
    let characters: Vec<char> = text.chars().collect();
    let found = occurrences(&characters, words);
    let first = found.first().map_or(0, |&(start, _)| start);
    let mut line_start = characters[..first]
        .iter()
        .rposition(|&character| character == '\n')
        .map_or(0, |newline| newline + 1);
    let mut line_end = characters[first..]
        .iter()
        .position(|&character| character == '\n')
        .map_or(characters.len(), |newline| first + newline);
    while line_start < line_end && characters[line_start].is_whitespace() {
        line_start += 1;
    }
    while line_end > line_start && characters[line_end - 1].is_whitespace() {
        line_end -= 1;
    }
    let (mut start, mut end) = (line_start, line_end);
    if end - start > LINE_MAX {
        start = first.saturating_sub(LEAD).max(line_start);
        end = (start + LINE_MAX).min(line_end);
        start = end.saturating_sub(LINE_MAX).max(line_start);
    }
    // The ellipses take the place of a character each, inside the bound.
    let cut_start = start > line_start;
    let cut_end = end < line_end;
    if cut_start {
        start += 1;
    }
    if cut_end && end > start {
        end -= 1;
    }
    let mut line = String::new();
    if cut_start {
        line.push('…');
    }
    line.extend(&characters[start..end]);
    if cut_end {
        line.push('…');
    }
    let ellipsis = if cut_start { '…'.len_utf16() } else { 0 };
    let ranges = utf16_ranges(&characters[start..end], ellipsis, &found, start);
    (line, ranges)
}

/// The places of every word in the whole title, as UTF-16 offsets, found
/// as a match line's are.
fn title_ranges(title: &str, words: &[String]) -> Vec<[u32; 2]> {
    let characters: Vec<char> = title.chars().collect();
    let found = occurrences(&characters, words);
    utf16_ranges(&characters, 0, &found, 0)
}

/// `found`, character positions in a text, as UTF-16 offsets in `shown`, the
/// piece of the text that starts at character `start` and follows `lead`
/// UTF-16 units of its own, such as an ellipsis; pieces outside it are cut.
fn utf16_ranges(shown: &[char], lead: usize, found: &[(usize, usize)], start: usize) -> Vec<[u32; 2]> {
    let end = start + shown.len();
    let offset = |position: usize| -> u32 {
        let before: usize = shown[..position - start]
            .iter()
            .map(|character| character.len_utf16())
            .sum();
        u32::try_from(lead + before).expect("a line of 256 characters fits u32")
    };
    found
        .iter()
        .filter(|&&(from, to)| from < end && to > start)
        .map(|&(from, to)| [offset(from.max(start)), offset(to.min(end))])
        .collect()
}

/// Where the words occur in `characters`, as character positions, end
/// exclusive, in order, overlapping ones joined.
fn occurrences(characters: &[char], words: &[String]) -> Vec<(usize, usize)> {
    let mut found = Vec::new();
    for word in words {
        let word: Vec<char> = word.chars().collect();
        if word.is_empty() || word.len() > characters.len() {
            continue;
        }
        for start in 0..=characters.len() - word.len() {
            let matches = word
                .iter()
                .zip(&characters[start..])
                .all(|(&wanted, &character)| same_letter(wanted, character));
            if matches {
                found.push((start, start + word.len()));
            }
        }
    }
    found.sort_unstable();
    let mut joined: Vec<(usize, usize)> = Vec::new();
    for (start, end) in found {
        match joined.last_mut() {
            Some(last) if start <= last.1 => last.1 = last.1.max(end),
            _ => joined.push((start, end)),
        }
    }
    joined
}

/// Whether two characters are the same letter, case ignored.
fn same_letter(a: char, b: char) -> bool {
    a == b || a.to_lowercase().eq(b.to_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(text: &str, query: &str) -> (String, Vec<[u32; 2]>) {
        let words: Vec<String> = query.split_whitespace().map(str::to_owned).collect();
        match_line(text, &words)
    }

    #[test]
    fn a_match_line_is_the_line_around_the_first_match_with_its_words_in_utf16() {
        // The line that holds the first match, case ignored, every word
        // marked, overlapping ones as one.
        assert_eq!(
            line("intro\n  Fix TS2307 by fixing ts paths  \nmore", "ts2307 ts"),
            ("Fix TS2307 by fixing ts paths".into(), vec![[4, 10], [21, 23]])
        );
        // Offsets count UTF-16 units: an emoji takes two.
        assert_eq!(
            line("🙂 模块路径不对", "路径"),
            ("🙂 模块路径不对".into(), vec![[5, 7]])
        );
        // A long line is cut around the match, an ellipsis at each cut,
        // 160 characters in all.
        let long = format!("{}needle{}", "a".repeat(300), "b".repeat(300));
        let (cut, ranges) = line(&long, "NEEDLE");
        assert_eq!(cut.chars().count(), LINE_MAX);
        assert!(cut.starts_with('…') && cut.ends_with('…'));
        assert_eq!(ranges, vec![[40, 46]]);
        assert_eq!(&cut[ranges[0][0] as usize + 2..ranges[0][1] as usize + 2], "needle");
        // Near the start, the line keeps its start and is cut at the end.
        let (cut, ranges) = line(&format!("needle {}", "c".repeat(400)), "needle");
        assert!(cut.starts_with("needle") && cut.ends_with('…'));
        assert_eq!(ranges, vec![[0, 6]]);
    }
}
