//! Finding a conversation (`storage.md` § Search index): the user's indexer, which keeps
//! the user's search index in step with the conversations, and the search the edge
//! answers from that index alone.
//!
//! The indexer follows each save of a root's checkpoint by what it changed:
//! the save hands over the version it follows and the one it makes, the
//! searchable text of the blocks it wrote and where it cut the rest, and the
//! indexer applies the saves of each conversation in turn, without reading
//! the conversation, each only on top of the version it follows. Everything
//! else that changes a conversation's summary, among them a title change and
//! a deletion, marks the conversation on the indexer's watch; a marked
//! conversation is looked at once, and a mark within the interval after that
//! waits for its end. A mark reads the conversation's record and what the
//! index holds of it, and replaces the title's row when the title differs.
//! A conversation is read whole only when the index does not hold it: a mark
//! finds it missing, a save finds it at another version than the one the
//! save follows, the start finds it at another version, or the index is
//! reset. A whole read that fails leaves the index behind, so the next save
//! or start reads the conversation whole again.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_database::conversation_index::ConversationRecord;
use demi_backend_database::search::{IndexedConversation, TranscriptChange, title_ranges};
use demi_backend_database::tree;
use demi_backend_page_sync::{Part, Registration};
use demi_shared_gates::KeyedSerialGate;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use demi_web_api_protocol::search::{RESULTS_MAX, SearchMatch, SearchResult, SearchText};
use tokio::sync::broadcast::{self, error::RecvError};

use crate::services::Services;
use crate::shard::{Shard, Shards};

/// The user's indexer: the watch of the user's changes, one indexing at a
/// time per conversation, the conversations marked since their last look
/// began, the saves waiting to be applied, and where the catching up of
/// every conversation stands.
pub(crate) struct SearchIndexer {
    watch: Registration,
    turns: KeyedSerialGate<ConversationId>,
    /// Each conversation being looked at or waiting out its interval, with
    /// whether it was marked again meanwhile.
    marked: RefCell<HashMap<ConversationId, Rc<Cell<bool>>>>,
    /// Each conversation whose saves are being applied, with those not yet
    /// applied, oldest first.
    saves: RefCell<HashMap<ConversationId, VecDeque<TranscriptChange>>>,
    catching_up: Cell<CatchUp>,
}

/// Where the catching up of every conversation stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CatchUp {
    Idle,
    Running,
    /// Running, and asked for again, as a reset during it does.
    Again,
}

impl SearchIndexer {
    pub(crate) fn new(services: &Services, user: &UserId) -> Self {
        Self {
            watch: services.sync.watch(user),
            turns: KeyedSerialGate::new(),
            marked: RefCell::default(),
            saves: RefCell::default(),
            catching_up: Cell::new(CatchUp::Idle),
        }
    }
}

impl Shard {
    /// Looks at each conversation the user's changes mark, until the shard
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

    /// Applies what a save of one of the conversation's agents changed, after the
    /// saves before it, or reads the conversation whole when the index does
    /// not hold the version the change follows.
    pub(crate) fn follow_save(&self, id: ConversationId, change: TranscriptChange) {
        let indexer = self.search_indexer();
        if let Some(waiting) = indexer.saves.borrow_mut().get_mut(&id) {
            waiting.push_back(change);
            return;
        }
        indexer.saves.borrow_mut().insert(id.clone(), VecDeque::from([change]));
        let shard = self.this();
        self.tasks().spawn_local(async move {
            loop {
                let _turn = shard.search_indexer().turns.acquire(id.clone()).await;
                let next = {
                    let mut saves = shard.search_indexer().saves.borrow_mut();
                    let next = saves.get_mut(&id).and_then(VecDeque::pop_front);
                    if next.is_none() {
                        saves.remove(&id);
                    }
                    next
                };
                let Some(change) = next else {
                    return;
                };
                let services = shard.services();
                let applied = services.search.apply(shard.user(), &id, change).await;
                match applied {
                    Ok(true) => continue,
                    Ok(false) => {}
                    Err(error) => tracing::warn!(
                        conversation = %id,
                        error = &error as &dyn std::error::Error,
                        "a save was not indexed for search; the conversation is read whole"
                    ),
                }
                // The index lacks the conversation, or a change before this
                // one: it is read whole, unless it is at the conversation's
                // version already, which holds the saves waiting so far. A
                // failed read leaves the index behind the conversation, so
                // the next save, or the next start, reads it whole again.
                let caught_up = match services.control.conversation(id.clone()).await {
                    Ok(record) => shard.catch_up(&id, record, true).await,
                    Err(error) => Err(error),
                };
                report_catch_up(&id, caught_up);
            }
        });
    }

    /// Looks at the conversation now, unless a look at it began within the
    /// interval: then once the interval has passed.
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
                    Ok(record) => {
                        let _turn = shard.search_indexer().turns.acquire(id.clone()).await;
                        shard.catch_up(&id, record, false).await
                    }
                    Err(error) => Err(error),
                };
                report_catch_up(&id, indexed);
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
    /// `record`, its record now, in the conversation's indexing turn, which
    /// the caller holds: removed when it is gone or not the user's; read
    /// whole when the index does not hold it, or, with `versions`, holds any
    /// agent's transcript at another saved version than it has; otherwise its
    /// title's row replaced when the title differs.
    async fn catch_up(
        &self,
        id: &ConversationId,
        record: Option<ConversationRecord>,
        versions: bool,
    ) -> Result<(), StorageError> {
        let services = self.services();
        let user = self.user();
        let Some(record) = record.filter(|record| record.owner == *user) else {
            return services.search.remove_conversation(user, id).await;
        };
        // The placeholder holds no words, which no query finds.
        let title = record.title.unwrap_or_default();
        let indexed = services.search.indexed(user, &record.id).await?;
        let whole = match &indexed {
            None => true,
            Some(_) if !versions => false,
            Some(indexed) => {
                let versions = services
                    .conversations
                    .read(&record.id, tree::search_versions)
                    .await?
                    .unwrap_or_default();
                indexed.versions != versions
            }
        };
        if !whole {
            if indexed.is_some_and(|indexed| indexed.title != title) {
                services.search.retitle(user, &record.id, title).await?;
            }
            return Ok(());
        }
        // The saves waiting so far committed before the read below, which
        // holds them; one that commits later is applied after it.
        if let Some(waiting) = self.search_indexer().saves.borrow_mut().get_mut(&record.id) {
            waiting.clear();
        }
        let agents = services
            .conversations
            .read(&record.id, tree::search_texts)
            .await?
            .unwrap_or_default();
        let conversation = IndexedConversation { title, agents };
        services.search.index(user, &record.id, conversation).await
    }

    /// Brings every conversation of the user's in step, in the background,
    /// at start and after the index is reset: each of the user's
    /// conversations, the most recently active first, and then the rows of
    /// any conversation that is gone. An index built again is empty, so this
    /// fills it. Asked for while it runs, it runs once more after.
    pub(crate) async fn catch_up_everything(self: Rc<Self>) {
        let indexer = self.search_indexer();
        if indexer.catching_up.get() != CatchUp::Idle {
            indexer.catching_up.set(CatchUp::Again);
            return;
        }
        loop {
            indexer.catching_up.set(CatchUp::Running);
            if let Err(error) = self.index_everything().await {
                // The conversations not reached are indexed by their next
                // change, or at the next start.
                tracing::warn!(
                    user = %self.user(),
                    error = &error as &dyn std::error::Error,
                    "the search index was not brought in step"
                );
            }
            if indexer.catching_up.get() != CatchUp::Again || self.is_closing() {
                break;
            }
        }
        indexer.catching_up.set(CatchUp::Idle);
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
            let _turn = self.search_indexer().turns.acquire(id.clone()).await;
            let caught_up = self.catch_up(&id, Some(record), true).await;
            report_catch_up(&id, caught_up);
        }
        for id in services.search.conversations(user).await? {
            if !present.contains(&id) {
                let _turn = self.search_indexer().turns.acquire(id.clone()).await;
                let caught_up = self.catch_up(&id, None, true).await;
                report_catch_up(&id, caught_up);
            }
        }
        Ok(())
    }
}

/// Logs a conversation the catching up of every conversation could not
/// bring in step, which goes on with the others: the index stays behind it,
/// so its next save, or the next start, reads it whole again.
fn report_catch_up(id: &ConversationId, caught_up: Result<(), StorageError>) {
    if let Err(error) = caught_up {
        tracing::warn!(
            conversation = %id,
            error = &error as &dyn std::error::Error,
            "a conversation was not indexed for search"
        );
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
        let handed = shards.of(&owner).adopt(Shard::catch_up_everything).await;
        if handed.is_err() {
            // The backend is shutting down; the next start indexes what is
            // left.
            return Ok(());
        }
    }
    Ok(())
}

/// Has each user whose search index is reset, from `resets`, index every
/// conversation again as at start, until the backend closes. A follower that
/// fell behind and missed some resets has every user do so.
pub async fn follow_resets(
    mut resets: broadcast::Receiver<UserId>,
    control: ControlService,
    shards: Shards,
) {
    loop {
        let handed = match resets.recv().await {
            Ok(user) => shards.of(&user).adopt(Shard::catch_up_everything).await.is_ok(),
            Err(RecvError::Lagged(_)) => match index_at_start(&control, &shards).await {
                Ok(()) => true,
                Err(error) => {
                    // The users not reached are brought in step at the next
                    // start; their conversations' changes meanwhile apply as
                    // they come.
                    tracing::warn!(
                        error = &error as &dyn std::error::Error,
                        "the conversations to index for search cannot be listed"
                    );
                    true
                }
            },
            Err(RecvError::Closed) => false,
        };
        if !handed {
            // The backend is shutting down.
            return;
        }
    }
}

/// Answers `query` for `user` from the user's index alone: no
/// conversation's database is opened and no Host woken. The conversations'
/// records give their current titles, archive state and activity, and their
/// order: title matches first, then the rest, each kind most recently
/// active first; a conversation whose record is gone is left out.
pub async fn search(
    services: &Services,
    user: &UserId,
    query: &SearchText,
) -> Result<Vec<SearchResult>, StorageError> {
    let words = query.words();
    // The index holds each conversation under its record's spelling.
    let mut records: Vec<ConversationRecord> = Vec::new();
    for archived in [false, true] {
        records.extend(services.control.conversations(user.clone(), archived).await?);
    }
    records.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then_with(|| a.id.cmp(&b.id)));
    let ranked = records.iter().map(|record| record.id.clone()).collect();
    let mut records: HashMap<ConversationId, ConversationRecord> =
        records.into_iter().map(|record| (record.id.clone(), record)).collect();
    let found = services.search.find(user, words.clone(), ranked, RESULTS_MAX).await?;
    Ok(found
        .into_iter()
        .filter_map(|found| {
            let record = records.remove(&found.conversation)?;
            let r#match = found.message.map(|message| SearchMatch {
                block_id: message.block,
                text: message.line,
                ranges: message.ranges,
            });
            let title_ranges = if found.title {
                title_ranges(record.title.as_deref().unwrap_or_default(), &words)
            } else {
                Vec::new()
            };
            Some(SearchResult {
                conversation_id: record.id,
                title_ranges,
                title: record.title,
                archived: record.archived,
                last_active_at: record.updated_at,
                r#match,
            })
        })
        .collect())
}
