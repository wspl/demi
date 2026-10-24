//! Finding a conversation, `GET /search?q=`, and finding in one,
//! `GET /conversations/{id}/search?q=`, and what each answers.

use demi_shared_types::{BlockId, Nullable, Timestamp, trim};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::ConversationId;

/// The most conversations, or messages of one conversation, one search
/// answers.
///
/// Bounded because a search answers whole, and the conversations, as the
/// messages of one, have no end.
pub const RESULTS_MAX: usize = 50;

/// The most characters (Unicode scalar values) of a match's line.
///
/// Bounded because a line of a result goes to the page whole, and a line has no
/// length of its own.
pub const LINE_MAX: usize = 160;

/// `?q=`: what the user typed.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchQuery {
    pub q: SearchText,
}

/// A query as the backend reads it: trimmed, with at least one
/// character. Its words are the pieces between white space.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct SearchText(String);

/// Why a text is not a query.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a query has a character besides the white space around it")]
pub struct NotQuery;

impl TryFrom<String> for SearchText {
    type Error = NotQuery;

    fn try_from(text: String) -> Result<Self, NotQuery> {
        let trimmed = trim(&text);
        if trimmed.is_empty() {
            return Err(NotQuery);
        }
        Ok(Self(trimmed.to_owned()))
    }
}

impl SearchText {
    /// The words, each once, in the order typed.
    pub fn words(&self) -> Vec<String> {
        let mut words: Vec<String> = Vec::new();
        for word in self.0.split_whitespace() {
            if !words.iter().any(|seen| seen == word) {
                words.push(word.to_owned());
            }
        }
        words
    }
}

/// What `GET /search` answers: the caller's conversations that match, title
/// matches first, then the others, each group most recently active first.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SearchResults {
    pub results: Vec<SearchResult>,
}

/// A conversation the query found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    pub conversation_id: ConversationId,
    /// Its title; null while it is the placeholder, as the conversation's
    /// summary has it.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<String>")]
    pub title: Option<String>,
    /// The places of the query's words in the title, as `SearchMatch`'s
    /// ranges give them; empty when the title does not match.
    pub title_ranges: Vec<[u32; 2]>,
    pub archived: bool,
    /// When the conversation's latest message was written.
    pub last_active_at: Timestamp,
    /// The message that matched; null when only the title does.
    #[serde(deserialize_with = "Option::deserialize")]
    #[schemars(with = "Nullable<SearchMatch>")]
    pub r#match: Option<SearchMatch>,
}

/// The newest message that holds every word of the query: its block, the
/// line of its text around the first match, and where the words are in that
/// line, as `[start, end]` offsets in UTF-16 code units, end exclusive.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchMatch {
    pub block_id: BlockId,
    pub text: String,
    pub ranges: Vec<[u32; 2]>,
}

/// What `GET /conversations/{id}/search` answers: the messages of the
/// conversation's main agent, the user's and the agent's answers, that hold
/// every word of the query, newest first, at most [`RESULTS_MAX`] of them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MessageResults {
    pub messages: Vec<MessageResult>,
    /// More messages hold the words than the answer lists.
    pub limited: bool,
}

/// A message of the conversation the query found: its block, who wrote it
/// and when, and its line around the first match with where the words are,
/// as `SearchMatch` gives them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MessageResult {
    pub block_id: BlockId,
    pub author: MessageAuthor,
    pub time: Timestamp,
    pub text: String,
    pub ranges: Vec<[u32; 2]>,
}

/// Who wrote a message: the user, or the agent, whose answer it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub enum MessageAuthor {
    User,
    Agent,
}
