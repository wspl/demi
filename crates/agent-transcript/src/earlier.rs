//! What a session keeps of its path's blocks before its window
//! (`runtime.md` § What a session holds): the few facts its decisions read
//! about the whole path, so that answering them never needs the blocks.
//! A block the window lets go of is folded in; a restore reads them from
//! the store.

use std::collections::HashSet;

use demi_agent_store::{EarlierRows, SessionStore, StoreError};
use demi_shared_types::{AutomationMark, Block, BlockId, ErrorBlock, Timestamp, TurnId};

use super::cut::{turn_of, turn_record};

/// The latest message the user or an automation wrote, as
/// [`Earlier::user_input`] tells it: the automation's run and when it came
/// for a run's message, none for the user's own.
pub type UserInput = Option<(AutomationMark, Timestamp)>;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Earlier {
    user_turns: HashSet<TurnId>,
    calls: HashSet<BlockId>,
    last_turn: Option<TurnId>,
    /// The record of the last turn there ([`turn_record`]): the failure that
    /// left it unfinished, or none for any other record.
    record: Option<Option<ErrorBlock>>,
    user_input: Option<UserInput>,
}

impl Earlier {
    /// Adds what `blocks`, which follow the blocks it kept, tell.
    pub fn fold(&mut self, blocks: &[Block]) {
        for block in blocks {
            match block {
                Block::User(user) => {
                    self.user_turns.insert(user.turn_id.clone());
                }
                Block::ToolCall(call) => {
                    self.calls.insert(call.id.clone());
                }
                _ => {}
            }
        }
        if let Some(turn) = blocks.iter().rev().find_map(turn_of) {
            self.last_turn = Some(turn.clone());
        }
        if let Some(record) = turn_record(blocks) {
            self.record = Some(failure(record));
        }
        if let Some(input) = blocks.iter().rev().find_map(user_input) {
            self.user_input = Some(input);
        }
    }

    /// What the path's blocks before `end` tell, read from `store` in one
    /// read without holding them.
    pub async fn read(store: &dyn SessionStore, end: usize) -> Result<Self, StoreError> {
        Ok(Self::of(store.earlier(end).await?))
    }

    /// What `rows`, as the store read them of the blocks before a place,
    /// tell.
    pub fn of(rows: EarlierRows) -> Self {
        Self {
            user_turns: rows.user_turns.into_iter().collect(),
            calls: rows.calls.into_iter().collect(),
            last_turn: rows.last_input.as_ref().and_then(turn_of).cloned(),
            record: rows.last_record.as_ref().map(failure),
            user_input: rows.last_user_input.as_ref().and_then(user_input),
        }
    }

    pub fn has_user_turn(&self, turn: &TurnId) -> bool {
        self.user_turns.contains(turn)
    }

    pub fn holds_call(&self, id: &BlockId) -> bool {
        self.calls.contains(id)
    }

    /// The turn of the last block of a turn's input.
    pub fn last_turn(&self) -> Option<&TurnId> {
        self.last_turn.as_ref()
    }

    /// The record of the last turn: whether there is one, and the failure
    /// it is when it is an `error` block.
    pub fn record(&self) -> Option<Option<&ErrorBlock>> {
        self.record.as_ref().map(Option::as_ref)
    }

    /// The latest message the user or an automation wrote, when there is
    /// one.
    pub fn user_input(&self) -> Option<&UserInput> {
        self.user_input.as_ref()
    }
}

/// The failure a turn's record is, when it is an `error` block.
fn failure(record: &Block) -> Option<ErrorBlock> {
    match record {
        Block::Error(error) => Some(error.clone()),
        _ => None,
    }
}

/// What a `user` block or a steer tells of who wrote it.
pub fn user_input(block: &Block) -> Option<UserInput> {
    match block {
        Block::User(user) => Some(user.automation.clone().map(|mark| (mark, user.created_at))),
        Block::Steer(_) => Some(None),
        _ => None,
    }
}
