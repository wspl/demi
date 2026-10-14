//! What the conversation shows of a transcript's work, computed once as a
//! block is stored and kept with it (`runtime.md` § Work groups,
//! `edit-tracking.md` § What the conversation shows): the run of steps each
//! block belongs to, and for each shell call its own files, its run's and
//! its request's. The session applies them to the blocks it writes and the
//! conversation database's migration to the blocks it stored before, with
//! this one code, so a page counts and derives nothing.
//!
//! A list is a function of the blocks up to its call: where a run or a
//! request starts is decided by the blocks before it, and later blocks only
//! end it.

use std::collections::{HashMap, HashSet};

use bytes::Bytes;
use demi_command_protocol::line_counts;
use demi_shared_types::{
    AgentMessageEvent, B64Bytes, BlobRef, Block, BlockId, ChangedFile, CommandId, EditKind, EditedFile,
    FileEdit, LineCounts, PathChange, ReportEvent, ShellToolView, ToolView,
};

use crate::media::BlobStore;

/// How a block stands toward a run of steps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Place {
    /// A step: thinking, a call, a command's notice or an agent's receipt.
    Step,
    /// A block the reader never sees, which neither joins nor ends a run:
    /// among them a `resume` and a Stop the user continued, so the steps
    /// after a Continue, a Resume or the system's resume after a restart
    /// join the run before it.
    Passes,
    /// What the reader reads, which ends a run: the agent's text, the
    /// user's words and decisions, an error, a stop, a compaction's divider.
    Ends,
}

fn place(block: &Block) -> Place {
    match block {
        Block::Thinking(_) | Block::RedactedThinking(_) | Block::ToolCall(_) => Place::Step,
        // A block of progress reports alone shows nothing.
        Block::Wakeup(wakeup) => {
            if wakeup
                .reports
                .iter()
                .any(|report| !matches!(report.event, ReportEvent::Running { .. }))
            {
                Place::Step
            } else {
                Place::Passes
            }
        }
        // Another agent's update or end is a step; the receipts of the
        // user's decisions and Demi's notices end a run, as the user's words
        // do.
        Block::AgentMessage(message) => match message.message.event {
            AgentMessageEvent::Message {} | AgentMessageEvent::Completion { .. } => Place::Step,
            AgentMessageEvent::Permission { .. }
            | AgentMessageEvent::Moved { .. }
            | AgentMessageEvent::MoveFailed { .. } => Place::Ends,
        },
        // A boundary shows only where an edit removed its marker, which an
        // edit does only by cutting every block after it; its marker is the
        // divider that ends the run.
        Block::Response(_)
        | Block::Resume(_)
        | Block::Context(_)
        | Block::CompactionBoundary(_) => Place::Passes,
        // A Stop is continued before any step after it is stored, so the
        // run a later step joins never changes.
        Block::Abort(abort) if abort.is_resumed => Place::Passes,
        Block::User(_)
        | Block::Steer(_)
        | Block::Abort(_)
        | Block::Text(_)
        | Block::Error(_)
        | Block::CompactionMarker(_) => Place::Ends,
    }
}

/// The run `block` joins after `before` (`runtime.md` § Work groups): the
/// run of the step before it, past the blocks the reader never sees, or a
/// run of its own after anything that ends one; none for a block that is
/// not a step.
pub fn run_of(before: &[Block], block: &Block) -> Option<BlockId> {
    if place(block) != Place::Step {
        return None;
    }
    for previous in before.iter().rev() {
        match place(previous) {
            Place::Passes => continue,
            Place::Ends => break,
            Place::Step => {
                return Some(previous.run().unwrap_or(previous.id()).clone());
            }
        }
    }
    Some(block.id().clone())
}

/// Gives every block of `blocks` its run, in order, as storing them one by
/// one would.
pub fn set_runs(blocks: &mut [Block]) {
    for index in 0..blocks.len() {
        let run = run_of(&blocks[..index], &blocks[index]);
        blocks[index].set_run(run);
    }
}

/// The two ends a file's lines are counted between: its contents before
/// the list's first edit of it, and after its last, none once it is gone.
type Ends = (BlobRef, Option<BlobRef>);

/// A file as a list follows it across calls.
#[derive(Debug, Clone)]
struct Tracked {
    path: String,
    /// It was there before the list's first call: the list did not create it.
    existed: bool,
    /// A later call removed it.
    removed: bool,
    /// Where it was before the list's calls renamed it.
    from: Option<String>,
    edits: Vec<FileEdit>,
}

impl Tracked {
    fn kind(&self) -> EditKind {
        if self.removed {
            EditKind::Deleted
        } else if !self.existed {
            EditKind::Added
        } else if self.from.is_some() {
            EditKind::Renamed
        } else {
            EditKind::Modified
        }
    }
}

/// Whether `path` is `parent` or within it, a path of the Host with either
/// separator, as a Windows Host names its paths with `\\`.
fn is_within(path: &str, parent: &str) -> bool {
    let parent = parent.trim_end_matches(['/', '\\']);
    path == parent
        || path
            .strip_prefix(parent)
            .is_some_and(|rest| rest.starts_with(['/', '\\']))
}

/// Which paths a shown list leaves out: those outside the working
/// directory of the agent whose call it is (`edit-tracking.md` § Within the
/// working directory).
enum Outside<'a> {
    /// Outside `directory`.
    Directory(&'a str),
    /// The paths the calls' own lists marked `outside` as they were
    /// stored, for a list made again where the directory is not known.
    Marked(HashSet<&'a str>),
}

impl Outside<'_> {
    fn holds(&self, path: &str) -> bool {
        match self {
            Self::Directory(directory) => !is_within(path, directory),
            Self::Marked(paths) => paths.contains(path),
        }
    }
}

/// The files of a run of calls, as they stand after the latest.
#[derive(Debug, Default)]
struct Tracking {
    /// The calls by their command, numbered as they come.
    order: HashMap<CommandId, usize>,
    files: Vec<Tracked>,
}

impl Tracking {
    /// Adds a call: first its renames and removals, in the order it made
    /// them, move or drop the files the earlier calls changed; then its own
    /// files, which its job names as they stand after it, join them.
    fn add(&mut self, call: &CommandId, report: &[&EditedFile], changes: &[PathChange]) {
        let number = self.order.len();
        self.order.entry(call.clone()).or_insert(number);
        // Files a rename of this call brought to a path the list did not
        // hold: whether a file was there before, only the call's own entry
        // can tell.
        let mut arrived = HashSet::new();
        for change in changes {
            match change {
                PathChange::Removed { path } => self.remove(path),
                PathChange::Renamed { from, to } => self.rename(from, to, &mut arrived),
            }
        }
        for file in report {
            let at = match self.files.iter().position(|tracked| tracked.path == file.path) {
                Some(at) => at,
                None => {
                    self.files.push(Tracked {
                        path: file.path.clone(),
                        existed: file.kind == EditKind::Modified,
                        removed: false,
                        from: None,
                        edits: Vec::new(),
                    });
                    self.files.len() - 1
                }
            };
            let tracked = &mut self.files[at];
            if arrived.contains(&file.path) && file.kind == EditKind::Modified {
                // The rename replaced a file that was there.
                tracked.existed = true;
                tracked.from = None;
            }
            // Written again after a removal, the file is there again.
            tracked.removed = false;
            for (segment, edit) in file.edits.iter().enumerate() {
                tracked.edits.push(FileEdit {
                    call: call.clone(),
                    path: file.path.clone(),
                    segment: u32::try_from(segment).unwrap_or(u32::MAX),
                    created: file.kind == EditKind::Added && segment == 0,
                    copies: edit.copies.clone(),
                });
            }
        }
    }

    /// A removed path, or a folder with all in it: a file the list created
    /// leaves it, one that was there before is deleted where it was before
    /// the list renamed it, and a path the list did not hold is deleted.
    fn remove(&mut self, path: &str) {
        let mut within = false;
        self.files.retain_mut(|tracked| {
            if !is_within(&tracked.path, path) {
                return true;
            }
            within = true;
            if !tracked.existed {
                return false;
            }
            tracked.removed = true;
            if let Some(from) = tracked.from.take() {
                tracked.path = from;
            }
            true
        });
        if !within {
            self.files.push(Tracked {
                path: path.strip_suffix('/').unwrap_or(path).to_owned(),
                existed: true,
                removed: true,
                from: None,
                edits: Vec::new(),
            });
        }
    }

    /// A renamed path, or a folder with all in it: each file takes its new
    /// name with its edits, merged into the file the new name already has,
    /// which keeps its place, or the earlier of the two; a path the list
    /// did not hold is listed renamed.
    fn rename(&mut self, from: &str, to: &str, arrived: &mut HashSet<String>) {
        let moved: Vec<usize> = (0..self.files.len())
            .filter(|&at| !self.files[at].removed && is_within(&self.files[at].path, from))
            .collect();
        if moved.is_empty() {
            // A listed file it replaced stays listed, its new contents the
            // call's own edit of it.
            if self.files.iter().any(|tracked| tracked.path == to) {
                return;
            }
            self.files.push(Tracked {
                path: to.to_owned(),
                existed: true,
                removed: false,
                from: Some(from.to_owned()),
                edits: Vec::new(),
            });
            arrived.insert(to.to_owned());
            return;
        }
        // Later positions first, so removing one leaves the others' in place.
        for &at in moved.iter().rev() {
            let old = self.files[at].path.clone();
            let path = format!("{to}{}", &old[from.strip_suffix('/').unwrap_or(from).len()..]);
            let target = self.files.iter().position(|tracked| tracked.path == path);
            match target {
                None => {
                    let tracked = &mut self.files[at];
                    if tracked.existed {
                        let original = tracked.from.take().unwrap_or(old);
                        tracked.from = (original != path).then_some(original);
                    }
                    tracked.path = path.clone();
                    arrived.insert(path);
                }
                Some(target) => {
                    let moving = self.files[at].edits.clone();
                    let order = &self.order;
                    let rank = |edit: &FileEdit| order.get(&edit.call).copied().unwrap_or(0);
                    let kept = &mut self.files[target];
                    kept.edits.extend(moving);
                    // Each call's edits stay in their order, the replaced
                    // file's first.
                    kept.edits.sort_by_key(rank);
                    kept.removed = false;
                    let merged = self.files[target].clone();
                    let first = at.min(target);
                    self.files.remove(at.max(target));
                    self.files[first] = merged;
                }
            }
        }
    }

    /// The list a page shows: the files whose path, after the list's
    /// renames, `outside` does not leave out, each with the ends its lines
    /// are counted between.
    fn listed(&self, outside: &Outside<'_>) -> Vec<(ChangedFile, Option<Ends>)> {
        self.files
            .iter()
            .filter(|tracked| !outside.holds(&tracked.path))
            .map(|tracked| {
                let file = ChangedFile {
                    path: tracked.path.clone(),
                    kind: tracked.kind(),
                    from: tracked.from.clone(),
                    lines: None,
                    edits: tracked.edits.clone(),
                };
                let ends = ends(&file);
                (file, ends)
            })
            .collect()
    }
}

/// The ends a listed file's lines are counted between: from the original
/// of its first edit, or for a file that was there before, of its first
/// edit that did not create it, so a replaced page shows the old against
/// the new; to the result of its last edit, or nothing once it is deleted.
/// None when an end was not kept.
fn ends(file: &ChangedFile) -> Option<Ends> {
    let start = match file.kind {
        EditKind::Added => file.edits.first(),
        EditKind::Modified | EditKind::Renamed | EditKind::Deleted => file
            .edits
            .iter()
            .find(|edit| !edit.created)
            .or(file.edits.first()),
    }?;
    let original = start.copies.as_ref()?.original.clone();
    if file.kind == EditKind::Deleted {
        return Some((original, None));
    }
    let modified = file.edits.last()?.copies.as_ref()?.modified.clone();
    Some((original, Some(modified)))
}

/// What a call's job reported of its own: the files it wrote, without the
/// removals and renames its view lists beside them.
fn reported(view: &ShellToolView) -> Vec<&EditedFile> {
    view.files
        .iter()
        .flatten()
        .filter(|file| matches!(file.kind, EditKind::Added | EditKind::Modified))
        .collect()
}

/// A call's own files, its record of what it did that later lists are
/// made from and its pills show (`edit-tracking.md` § The report): the
/// files its job wrote, then each path it removed as `deleted` and each it
/// renamed, but to a path it wrote, as `renamed` with the old path; each
/// marked `outside` as `outside` says, which the pills leave out.
fn call_files(view: &ShellToolView, outside: &Outside<'_>) -> Vec<EditedFile> {
    let mut files: Vec<EditedFile> = reported(view).into_iter().cloned().collect();
    let written = files.len();
    for change in &view.path_changes {
        let (path, kind, from) = match change {
            PathChange::Removed { path } => {
                let path = path.strip_suffix('/').unwrap_or(path).to_owned();
                (path, EditKind::Deleted, None)
            }
            PathChange::Renamed { from, to } => (to.clone(), EditKind::Renamed, Some(from.clone())),
        };
        if files[..written].iter().any(|file| file.path == path) {
            continue;
        }
        // A later change of the same path says where it ended up.
        files.retain(|file| file.path != path);
        files.push(EditedFile {
            path,
            kind,
            from,
            added: 0,
            removed: 0,
            outside: false,
            edits: Vec::new(),
        });
    }
    for file in &mut files {
        file.outside = outside.holds(&file.path);
    }
    files
}

/// The shell view of `block`, if it is a shell call that has one.
fn shell_view(block: &Block) -> Option<&ShellToolView> {
    match block {
        Block::ToolCall(call) => match &call.view {
            Some(ToolView::Shell(view)) => Some(view),
            _ => None,
        },
        _ => None,
    }
}

/// The lists of one shell call, planned from the blocks before it, which
/// wait for the lines of the ends no earlier list counted.
#[derive(Debug)]
pub struct Planned {
    files: Vec<EditedFile>,
    run: Vec<(ChangedFile, Option<Ends>)>,
    /// None for a call before the transcript's first request.
    request: Option<Vec<(ChangedFile, Option<Ends>)>>,
    counted: HashMap<Ends, Option<LineCounts>>,
}

impl Planned {
    /// The ends whose lines are still to be counted.
    pub fn uncounted(&self) -> Vec<(BlobRef, Option<BlobRef>)> {
        let mut uncounted: Vec<Ends> = Vec::new();
        for (_, ends) in self.run.iter().chain(self.request.iter().flatten()) {
            if let Some(ends) = ends
                && !self.counted.contains_key(ends)
                && !uncounted.contains(ends)
            {
                uncounted.push(ends.clone());
            }
        }
        uncounted
    }

    /// Writes the lists into `view`, with the lines `counted` gives for the
    /// ends [`uncounted`](Self::uncounted) named; an end it has no count
    /// for leaves its file named alone.
    pub fn finish(
        self,
        view: &mut ShellToolView,
        counted: &HashMap<(BlobRef, Option<BlobRef>), Option<LineCounts>>,
    ) {
        let known = |ends: &Option<Ends>| -> Option<LineCounts> {
            let ends = ends.as_ref()?;
            self.counted
                .get(ends)
                .or_else(|| counted.get(ends))
                .copied()
                .flatten()
        };
        let list = |files: &[(ChangedFile, Option<Ends>)]| -> Vec<ChangedFile> {
            files
                .iter()
                .map(|(file, ends)| ChangedFile {
                    lines: known(ends),
                    ..file.clone()
                })
                .collect()
        };
        view.files = (!self.files.is_empty()).then(|| self.files.clone());
        view.run_files = Some(list(&self.run));
        view.request_files = self.request.as_deref().map(list);
    }
}

/// Plans the lists of the shell call at `index` of `blocks` whose view is
/// `view`, from the blocks before it: the call's own files, its run's and
/// its request's up to it, without the paths outside `directory`, the
/// working directory of the agent whose transcript it is
/// (`edit-tracking.md` § Within the working directory). Where `directory`
/// is not known, as for a list made again of a stored transcript, the
/// paths the calls marked `outside` as they were stored are left out. The
/// call's block need not hold `view` yet; the calls after it play no part.
pub fn plan(blocks: &[Block], index: usize, view: &ShellToolView, directory: Option<&str>) -> Planned {
    let before = &blocks[..index];
    let run = blocks[index]
        .run()
        .cloned()
        .or_else(|| run_of(before, &blocks[index]));
    let start = before.iter().rposition(|block| matches!(block, Block::User(_)));
    let mut request = Tracking::default();
    let mut runs = Tracking::default();
    // Lines the request's earlier lists counted, by their ends.
    let mut counted = HashMap::new();
    for block in &before[start.unwrap_or(0)..] {
        let Some(earlier) = shell_view(block) else {
            continue;
        };
        let changes = &earlier.path_changes;
        request.add(&earlier.command_id, &reported(earlier), changes);
        if run.is_some() && block.run() == run.as_ref() {
            runs.add(&earlier.command_id, &reported(earlier), changes);
        }
        for file in earlier
            .run_files
            .iter()
            .flatten()
            .chain(earlier.request_files.iter().flatten())
        {
            if let Some(ends) = ends(file) {
                counted.insert(ends, file.lines);
            }
        }
    }
    request.add(&view.command_id, &reported(view), &view.path_changes);
    runs.add(&view.command_id, &reported(view), &view.path_changes);
    let outside = match directory {
        Some(directory) => Outside::Directory(directory),
        None => Outside::Marked(
            before
                .iter()
                .filter_map(shell_view)
                .chain([view])
                .flat_map(|view| view.files.iter().flatten())
                .filter(|file| file.outside)
                .map(|file| file.path.as_str())
                .collect(),
        ),
    };
    Planned {
        files: call_files(view, &outside),
        run: runs.listed(&outside),
        request: start.map(|_| request.listed(&outside)),
        counted,
    }
}

/// Counts the lines between each pair of ends, reading each blob once from
/// `blobs`. An end the namespace does not hold, or that could not be read,
/// leaves its pair uncounted and its file named alone, as one whose ends
/// were not kept: the list is never lost over its counts.
pub async fn count(
    ends: Vec<(BlobRef, Option<BlobRef>)>,
    blobs: &dyn BlobStore,
) -> HashMap<(BlobRef, Option<BlobRef>), Option<LineCounts>> {
    let mut read: HashMap<BlobRef, Option<Bytes>> = HashMap::new();
    for (original, modified) in &ends {
        for blob in std::iter::once(original).chain(modified) {
            if !read.contains_key(blob) {
                let bytes = blobs.get(blob).await.ok().flatten().map(B64Bytes::into_bytes);
                read.insert(blob.clone(), bytes);
            }
        }
    }
    ends.into_iter()
        .map(|(original, modified)| {
            let lines = count_ends(&original, modified.as_ref(), |blob| read.get(blob).cloned().flatten());
            ((original, modified), lines)
        })
        .collect()
}

/// The lines between two ends whose bytes `read` gives; none when one is
/// missing.
pub fn count_ends(
    original: &BlobRef,
    modified: Option<&BlobRef>,
    mut read: impl FnMut(&BlobRef) -> Option<Bytes>,
) -> Option<LineCounts> {
    let before = read(original)?;
    let after = match modified {
        Some(modified) => Some(read(modified)?),
        None => None,
    };
    let (added, removed) = line_counts(Some(&before), after.as_deref());
    Some(LineCounts {
        added: u32::try_from(added).unwrap_or(u32::MAX),
        removed: u32::try_from(removed).unwrap_or(u32::MAX),
    })
}

/// The shell calls after `index` in its request that hold lists, which a
/// call completing after them, as calls of one step do, changes: each
/// list is of the calls before it, whatever order they returned in.
pub fn later_in_request(blocks: &[Block], index: usize) -> Vec<usize> {
    let mut later = Vec::new();
    for (at, block) in blocks.iter().enumerate().skip(index + 1) {
        if matches!(block, Block::User(_)) {
            break;
        }
        if shell_view(block).is_some_and(|view| view.run_files.is_some()) {
            later.push(at);
        }
    }
    later
}

/// Gives each block of a stored transcript, in order, its run and each
/// shell call's view its lists, as the session gave them as it stored each
/// block, from the edit copies `read` gives: what a conversation stored
/// before the lists existed becomes what a new one stores (`storage.md`
/// § Schemas and migrations). A blob `read` fails on stops it with the
/// failure; one the namespace does not hold leaves its file named alone.
pub fn relist<E>(
    blocks: &mut [Block],
    mut read: impl FnMut(&BlobRef) -> Result<Option<Bytes>, E>,
) -> Result<(), E> {
    set_runs(blocks);
    for index in 0..blocks.len() {
        let Some(mut view) = shell_view(&blocks[index]).cloned() else {
            continue;
        };
        let planned = plan(blocks, index, &view, None);
        let mut counted = HashMap::new();
        for (original, modified) in planned.uncounted() {
            let mut failed = None;
            let lines = count_ends(&original, modified.as_ref(), |blob| {
                read(blob).unwrap_or_else(|error| {
                    failed = Some(error);
                    None
                })
            });
            if let Some(error) = failed {
                return Err(error);
            }
            counted.insert((original, modified), lines);
        }
        planned.finish(&mut view, &counted);
        if let Block::ToolCall(call) = &mut blocks[index] {
            call.view = Some(ToolView::Shell(view));
        }
    }
    Ok(())
}
