//! The conversation browser's tab routes (`web-api.md` § Conversation
//! browser tabs): the bodies around the operations the agent's `demi
//! browser` commands run, whose tab records they reuse.

use demi_builtin_protocol::browser::BrowserTab;
use garde::Validate;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The most characters of a URL a route takes.
pub const URL_MAX: usize = 4096;

/// `GET …/browser/tabs`: the browser's tabs; a browser that does not run
/// has none.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BrowserTabs {
    pub tabs: Vec<BrowserTab>,
}

/// `POST …/browser/tabs { url? }`: a new tab, at `about:blank` without a
/// URL.
#[derive(Debug, Default, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct OpenTab {
    #[serde(default)]
    #[garde(length(chars, min = 1, max = URL_MAX))]
    pub url: Option<String>,
}

/// `POST …/browser/tabs/:tab/navigate { url }`.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct NavigateTab {
    #[garde(length(chars, min = 1, max = URL_MAX))]
    pub url: String,
}

/// Where `POST …/browser/tabs/:tab/history` moves a tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HistoryAction {
    Back,
    Forward,
    Reload,
}

/// `POST …/browser/tabs/:tab/history { action }`.
#[derive(Debug, Deserialize, JsonSchema, Validate)]
#[serde(deny_unknown_fields)]
pub struct TabHistory {
    #[garde(skip)]
    pub action: HistoryAction,
}
