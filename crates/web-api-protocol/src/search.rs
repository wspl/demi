//! Finding a conversation (`web-api.md` § Search): `GET /search?q=` and the
//! conversations it answers.

use demi_shared_types::{BlockId, Nullable, Timestamp, trim};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::ids::ConversationId;

/// The most characters (Unicode scalar values) a query has, once trimmed.
pub const QUERY_MAX: usize = 256;

/// The most conversations one search answers.
pub const RESULTS_MAX: usize = 50;

/// The most characters (Unicode scalar values) of a match's line.
pub const LINE_MAX: usize = 160;

/// `?q=`: what the user typed.
#[derive(Debug, Clone, Deserialize)]
pub struct SearchQuery {
    pub q: SearchText,
}

/// A query as the backend reads it: trimmed, with 1 to `QUERY_MAX`
/// characters. Its words are the pieces between white space.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(try_from = "String")]
pub struct SearchText(String);

/// Why a text is not a query.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("a query has 1 to {QUERY_MAX} characters besides the white space around them")]
pub struct NotQuery;

impl TryFrom<String> for SearchText {
    type Error = NotQuery;

    fn try_from(text: String) -> Result<Self, NotQuery> {
        let trimmed = trim(&text);
        let length = trimmed.chars().count();
        if length == 0 || length > QUERY_MAX {
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
    pub title: String,
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

