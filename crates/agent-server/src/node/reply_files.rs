//! Which paths a reply's inline code names are files on the conversation's
//! Host, a fact of the reply (`docs/decisions/0009-agent-sessions-and-sync.md`).
//!
//! When a node's text becomes complete, its inline code spans that read as a
//! path, such as `docs/guide.md` or `src/app.rs:42`, are asked of the product
//! once, all in one question, and the answer is kept with the block as its
//! `codeFiles`, which a page shows as links. Nothing watches those paths
//! afterwards, and a page asks no Host: what the Host could not say then stays
//! code.
//!
//! ```text
//! text complete ──▶ code spans that read as paths ──▶ product: which are
//!                                                     files, looked at
//!                                                     without waking the Host
//!               ◀── block keeps `codeFiles` ◀─────────┘
//! ```

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::{Rc, Weak},
    sync::LazyLock,
};

use comrak::{Arena, Options, nodes::NodeValue, parse_document};
use demi_agent_session::{AgentSession, SessionEvent, Subscription};
use demi_agent_tools::HostResolver;
use demi_conversation_socket_protocol::TranscriptPatch;
use demi_shared_types::{Block, BlockId, CodeFile, NodeId, TextBlock};
use futures_util::future::LocalBoxFuture;
use regex::Regex;
use tokio::task::JoinSet;

use super::NodeRuntime;

/// The most paths one reply asks about.
pub const CODE_PATHS_MAX: usize = 200;

/// What the product says of the paths a reply names.
pub trait ReplyFiles {
    /// Which of `paths`, each as a reply wrote it and read against `cwd`, are
    /// regular files on the conversation `root`'s primary Host, by the path as
    /// written, each with the path the Host names it by. It looks without
    /// waking the Host; none when the Host could not say, as a stopped Cloud
    /// cannot.
    fn files<'a>(
        &'a self,
        root: &'a NodeId,
        cwd: &'a str,
        paths: Vec<String>,
    ) -> LocalBoxFuture<'a, Option<HashMap<String, String>>>;
}

/// A URL scheme of two or more letters; a single letter before `:` is a
/// drive.
static SCHEME: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^(?i)[a-z][a-z0-9+.-]+:").expect("a valid pattern"));
/// Characters a shell or a pattern gives meaning to, which a path written as
/// code does not hold.
static NOT_IN_PATH: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"[\s*?<>|"'`$;,\\]"#).expect("a valid pattern"));
/// A name with an extension: some of a name, a dot, and a suffix that starts
/// with a letter.
static NAME_WITH_EXTENSION: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[^./][^/]*\.[A-Za-z][A-Za-z0-9_-]*$").expect("a valid pattern"));
/// A drive's start, as `C:/`.
static DRIVE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^[A-Za-z]:/").expect("a valid pattern"));
/// A line after a path, as `:42`.
static LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r":([1-9][0-9]*)$").expect("a valid pattern"));

/// The path and line an inline code span names, as the agent wrote it: one
/// with a `/` or a name with an extension, with an optional `:line`, and no
/// white space; none for code that reads as no path, such as a bare name
/// (`runner`), a URL, a command or a directory (`src/`). A bare name with an
/// extension, such as `README.md`, names the file in the working directory.
pub fn code_path(code: &str) -> Option<(&str, Option<u64>)> {
    let (path, line) = match LINE.captures(code) {
        Some(found) => (
            &code[..found.get(0).expect("a match has its whole").start()],
            found[1].parse::<u64>().ok(),
        ),
        None => (code, None),
    };
    if path.is_empty()
        || NOT_IN_PATH.is_match(path)
        || SCHEME.is_match(path)
        || path.starts_with('-')
        || path.starts_with('~')
    {
        return None;
    }
    if path.contains(':') && !(DRIVE.is_match(path) && !path[2..].contains(':')) {
        return None;
    }
    if path.ends_with('/') || path.ends_with('.') {
        return None;
    }
    let named = if path.contains('/') {
        path.split('/').any(|part| !part.is_empty() && part != "." && part != "..")
    } else {
        NAME_WITH_EXTENSION.is_match(path)
    };
    named.then_some((path, line))
}

/// The inline code spans of the Markdown `text`, in order, as the page's
/// renderer reads them.
fn code_spans(text: &str) -> Vec<String> {
    let mut options = Options::default();
    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.cjk_friendly_emphasis = true;
    let arena = Arena::new();
    let root = parse_document(&arena, text, &options);
    root.descendants()
        .filter_map(|node| match &node.data.borrow().value {
            NodeValue::Code(code) => Some(code.literal.clone()),
            _ => None,
        })
        .collect()
}

/// What a complete reply's spans name: each span that reads as a path, with
/// its path and line, and the paths to ask about, each once, no more than
/// one question asks.
fn named_paths(text: &str) -> (Vec<(String, String, Option<u64>)>, Vec<String>) {
    let mut named = Vec::new();
    let mut paths: Vec<String> = Vec::new();
    for code in code_spans(text) {
        let Some((path, line)) = code_path(&code) else {
            continue;
        };
        let path = path.to_owned();
        if !paths.contains(&path) {
            if paths.len() == CODE_PATHS_MAX {
                continue;
            }
            paths.push(path.clone());
        }
        named.push((code, path, line));
    }
    (named, paths)
}

/// The texts `patches` complete that keep no answer yet.
fn completed(patches: &[TranscriptPatch]) -> Vec<TextBlock> {
    patches
        .iter()
        .filter_map(|patch| match patch {
            TranscriptPatch::Add { value: Block::Text(text), .. }
            | TranscriptPatch::ReplaceBlock { value: Block::Text(text), .. }
                if text.forkable && text.code_files.is_none() =>
            {
                Some(text.clone())
            }
            _ => None,
        })
        .collect()
}

/// Follows `session`, the node `runtime`'s, and keeps with each text it
/// completes which of its code spans name files. Dropping the answer stops
/// the following and every question still out.
pub(super) fn follow<H: HostResolver>(
    session: &AgentSession,
    runtime: Weak<NodeRuntime<H>>,
    files: Rc<dyn ReplyFiles>,
) -> Following {
    let asking: Rc<RefCell<JoinSet<()>>> = Rc::default();
    let subscription = session.subscribe({
        let asking = Rc::downgrade(&asking);
        let session = session.downgrade();
        move |event| {
            let SessionEvent::TranscriptChanged { patches, .. } = event else {
                return;
            };
            let texts = completed(patches);
            if texts.is_empty() {
                return;
            }
            let Some(asking) = asking.upgrade() else {
                return;
            };
            let mut asking = asking.borrow_mut();
            // The questions that ended are let go of before another starts.
            while asking.try_join_next().is_some() {}
            for text in texts {
                asking.spawn_local(ask(text, session.clone(), runtime.clone(), files.clone()));
            }
        }
    });
    Following {
        _subscription: subscription,
        _asking: asking,
    }
}

/// A node's following of its complete texts.
pub(crate) struct Following {
    _subscription: Subscription,
    _asking: Rc<RefCell<JoinSet<()>>>,
}

/// Asks which of `text`'s code spans name files, and keeps the answer with
/// it. A text whose spans name no path keeps nothing: it has no question,
/// and a change of the transcript for it would only cost its pages a
/// revision.
async fn ask<H: HostResolver>(
    text: TextBlock,
    session: demi_agent_session::WeakSession,
    runtime: Weak<NodeRuntime<H>>,
    files: Rc<dyn ReplyFiles>,
) {
    let (named, paths) = named_paths(&text.text);
    if paths.is_empty() {
        return;
    }
    let kept = {
        let Some(runtime) = runtime.upgrade() else {
            return;
        };
        let cwd = match runtime.placement().await {
            Ok(now) => runtime.directory_at(&now),
            Err(error) => {
                tracing::warn!(node = %runtime.node, %error, "a reply's code paths were not looked at");
                return keep(&session, &text.id, Vec::new());
            }
        };
        let found = files.files(&runtime.root, &cwd, paths).await.unwrap_or_default();
        let mut seen = HashSet::new();
        named
            .into_iter()
            .filter_map(|(code, path, line)| {
                let resolved = found.get(&path)?;
                seen.insert(code.clone()).then(|| CodeFile {
                    code,
                    path: resolved.clone(),
                    line,
                })
            })
            .collect()
    };
    keep(&session, &text.id, kept);
}

fn keep(session: &demi_agent_session::WeakSession, id: &BlockId, files: Vec<CodeFile>) {
    if let Some(session) = session.upgrade() {
        session.keep_code_files(id, files);
    }
}
