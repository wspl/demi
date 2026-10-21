//! What a complete reply says of the Host's files it names, kept with the
//! reply (`docs/decisions/0009-agent-sessions-and-sync.md`,
//! `docs/decisions/0046-a-reply-keeps-the-pictures-it-shows.md`).
//!
//! When a node's text becomes complete, two questions are asked of the
//! product, and each answer is kept with the block, so a page asks no Host:
//!
//! - Which of its inline code spans that read as a path, such as
//!   `docs/guide.md` or `src/app.rs:42`, are files: asked once, all in one
//!   question, and kept as the block's `codeFiles`, which a page shows as
//!   links. Nothing watches those paths afterwards: what the Host could not
//!   say then stays code.
//! - Its images that name a file, such as `![chart](out/chart.png)`: each is
//!   copied into the conversation's attachments, in the background, and the
//!   block keeps the copy beside the path the agent wrote, as its `pictures`.
//!   The transcript names every such picture as being copied in the step that
//!   completes the text (`TranscriptLog::complete_tail_text`), and each
//!   settles as its copy is in, or as why it could not be made. Only a text
//!   this node saw written is copied: a text that comes back into the
//!   window, as a version's switch brings it, is not. A picture whose copy
//!   was not settled when the backend stopped is copied again when the node
//!   starts.
//!
//! ```text
//! text complete ──▶ code spans that read as paths ──▶ product: which are
//!               │                                     files, looked at
//!               │                                     without waking the Host
//!               │   ◀── block keeps `codeFiles` ◀─────┘
//!               │
//!               └─▶ block names its pictures `copying` as it completes
//!                   └▶ product: copy each, in turn, from the Host the
//!                      agent ran on
//!                   ◀── each settles `copied` or `failed` ◀─┘
//! ```

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    rc::{Rc, Weak},
    sync::LazyLock,
};

use comrak::{Arena, nodes::NodeValue, parse_document};
use demi_agent_session::{AgentSession, SessionEvent, Subscription};
use demi_agent_tools::HostResolver;
use demi_agent_transcript::{has_scheme, markdown_options, picture_path};
use demi_conversation_socket_protocol::TranscriptPatch;
use demi_shared_types::{Block, BlockId, CodeFile, NodeId, PictureCopy, PictureFailure, TextBlock};
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

    /// Copies the file at `path`, as `picture_path` names it and read against
    /// `cwd`, from the Host `device` of the conversation `root` into its
    /// attachments, as the reply of `node` shows it, and answers the
    /// attachment's number, such as `a3`; or why it could not be copied. The
    /// Host is looked at without waking it, and only while it is `device`.
    fn copy_picture<'a>(
        &'a self,
        root: &'a NodeId,
        node: &'a NodeId,
        device: Option<&'a str>,
        cwd: &'a str,
        path: &'a str,
    ) -> LocalBoxFuture<'a, Result<String, PictureFailure>>;
}

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
        || has_scheme(path)
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
    let arena = Arena::new();
    let root = parse_document(&arena, text, &markdown_options());
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

/// What `patches` ask of the node: the texts that complete with a question
/// of their code still to ask, and those it saw written that complete naming
/// pictures to copy. `writing` holds the texts seen written and not yet
/// complete, so a text that arrives complete, as a version's switch brings it
/// back, has its pictures kept as they are.
fn asked_by(patches: &[TranscriptPatch], writing: &mut HashSet<BlockId>) -> (Vec<TextBlock>, Vec<TextBlock>) {
    let mut code = Vec::new();
    let mut pictures = Vec::new();
    for patch in patches {
        let (TranscriptPatch::Add { value: Block::Text(text), .. } | TranscriptPatch::ReplaceBlock { value: Block::Text(text), .. }) = patch
        else {
            continue;
        };
        if !text.forkable {
            writing.insert(text.id.clone());
            continue;
        }
        if text.code_files.is_none() {
            code.push(text.clone());
        }
        let copying = text
            .pictures
            .iter()
            .flatten()
            .any(|picture| picture.copy == PictureCopy::Copying);
        if copying && writing.remove(&text.id) {
            pictures.push(text.clone());
        }
    }
    (code, pictures)
}

/// The blocks that have a question out, so a block that changes while it is
/// answered, as each answer changes it, is not asked again.
#[derive(Default)]
struct Asked {
    code: HashSet<BlockId>,
    pictures: HashSet<BlockId>,
    /// The texts seen written, not yet complete.
    writing: HashSet<BlockId>,
}

/// Follows `session`, the node `runtime`'s, and keeps with each text it
/// completes which of its code spans name files and what became of the
/// pictures it shows. Dropping the answer stops the following and every
/// question still out.
pub(super) fn follow<H: HostResolver>(
    session: &AgentSession,
    runtime: Weak<NodeRuntime<H>>,
    files: Rc<dyn ReplyFiles>,
) -> Following {
    let asking: Rc<RefCell<JoinSet<()>>> = Rc::default();
    let asked: Rc<RefCell<Asked>> = Rc::default();
    let subscription = session.subscribe({
        let asking = Rc::downgrade(&asking);
        let asked = asked.clone();
        let session = session.downgrade();
        let runtime = runtime.clone();
        let files = files.clone();
        move |event| {
            let SessionEvent::TranscriptChanged { patches, .. } = event else {
                return;
            };
            let mut asked = asked.borrow_mut();
            let (code, pictures) = asked_by(patches, &mut asked.writing);
            if code.is_empty() && pictures.is_empty() {
                return;
            }
            let Some(asking) = asking.upgrade() else {
                return;
            };
            let mut asking = asking.borrow_mut();
            // The questions that ended are let go of before another starts.
            while asking.try_join_next().is_some() {}
            for text in code {
                if asked.code.insert(text.id.clone()) {
                    asking.spawn_local(ask(text, session.clone(), runtime.clone(), files.clone()));
                }
            }
            for text in pictures {
                if asked.pictures.insert(text.id.clone()) {
                    asking.spawn_local(copy(text, session.clone(), runtime.clone(), files.clone()));
                }
            }
        }
    });
    // The copies the node began before it stopped go on.
    {
        let mut asking = asking.borrow_mut();
        for text in session.unsettled_pictures() {
            if asked.borrow_mut().pictures.insert(text.id.clone()) {
                asking.spawn_local(copy(text, session.downgrade(), runtime.clone(), files.clone()));
            }
        }
    }
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

/// Copies the pictures of the Host `text` shows that are still being copied,
/// one after another, and settles each in the block as its copy is in.
///
/// The transcript named the pictures as being copied when it completed the
/// text, so a page that sees the complete text never takes a picture for one
/// that was never asked for. The copies are of the Host the node works on
/// when the copying starts, read against its working directory then.
async fn copy<H: HostResolver>(
    text: TextBlock,
    session: demi_agent_session::WeakSession,
    runtime: Weak<NodeRuntime<H>>,
    files: Rc<dyn ReplyFiles>,
) {
    let named: Vec<(String, String)> = text
        .pictures
        .iter()
        .flatten()
        .filter(|picture| picture.copy == PictureCopy::Copying)
        .filter_map(|picture| Some((picture.target.clone(), picture_path(&picture.target)?)))
        .collect();
    let Some(runtime) = runtime.upgrade() else {
        return;
    };
    let settle = |target: &str, copy: PictureCopy| {
        if let Some(session) = session.upgrade() {
            session.settle_picture(&text.id, target, copy);
        }
    };
    let (device, cwd) = match runtime.placement().await {
        Ok(now) => {
            let cwd = runtime.directory_at(&now);
            (now.device, cwd)
        }
        Err(error) => {
            tracing::warn!(node = %runtime.node, %error, "a reply's pictures were not copied");
            for (target, _) in &named {
                settle(target, PictureCopy::Failed { reason: PictureFailure::HostUnavailable });
            }
            return;
        }
    };
    for (target, path) in named {
        let copy = match files
            .copy_picture(&runtime.root, &runtime.node, device.as_deref(), &cwd, &path)
            .await
        {
            Ok(attachment) => PictureCopy::Copied { attachment },
            Err(reason) => PictureCopy::Failed { reason },
        };
        settle(&target, copy);
    }
}
