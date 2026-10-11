//! The lists of changed files a transcript's shell calls carry
//! (`edit-tracking.md` § What the conversation shows, § Temporary files):
//! a call's own, its run's and its request's, as a stored transcript is
//! listed in order.

use std::collections::HashMap;

use bytes::Bytes;
use demi_agent_store::{changes::relist, testing::test_model};
use demi_shared_types::{
    BlobRef, Block, BlockId, ChangedFile, CommandId, EditCopies, EditKind, EditSegment, EditedFile,
    LineCounts, PathChange, ShellToolView, ShellViewStatus, TextBlock, Timestamp, ToolCallBlock,
    ToolCallStatus, ToolView, TurnId, UserBlock,
};

// Cost: pure, under a millisecond each.

/// The texts of the edit copies a transcript names, by their blobs.
#[derive(Default)]
struct Copies(HashMap<BlobRef, Bytes>);

impl Copies {
    fn blob(&mut self, text: &str) -> BlobRef {
        let blob = BlobRef::of(text.as_bytes());
        self.0.insert(blob.clone(), Bytes::from(text.to_owned()));
        blob
    }

    /// One edit of a file, from `original` to `modified`.
    fn edit(&mut self, original: &str, modified: &str) -> EditSegment {
        EditSegment {
            copies: Some(EditCopies {
                original: self.blob(original),
                modified: self.blob(modified),
            }),
        }
    }

    /// Lists `blocks` as a stored transcript is listed.
    fn list(&self, blocks: &mut [Block]) {
        relist(blocks, |blob| Ok::<_, ()>(self.0.get(blob).cloned())).unwrap();
    }
}

fn id(value: &str) -> BlockId {
    BlockId::try_from(value).unwrap()
}

fn user(value: &str) -> Block {
    Block::User(UserBlock {
        entries: Vec::new(),
        id: id(value),
        turn_id: TurnId::try_from(value).unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        content: Vec::new(),
        preamble: None,
    })
}

fn text(value: &str) -> Block {
    Block::Text(TextBlock {
        entries: Vec::new(),
        id: id(value),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        text: value.into(),
        forkable: true,
    })
}

/// A file a job wrote.
fn written(path: &str, kind: EditKind, edits: Vec<EditSegment>) -> EditedFile {
    EditedFile {
        path: path.into(),
        kind,
        from: None,
        added: 0,
        removed: 0,
        temporary: path.starts_with("/tmp/"),
        edits,
    }
}

/// A shell call whose command `command` wrote `files` and made `changes`.
fn shell(command: &str, files: Vec<EditedFile>, changes: Vec<PathChange>) -> Block {
    Block::ToolCall(ToolCallBlock {
        run: None,
        entries: Vec::new(),
        id: id(command),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        tool_use_id: format!("use-{command}"),
        tool_name: "shell".into(),
        input: "{}".into(),
        status: ToolCallStatus::Completed,
        output: Vec::new(),
        view: Some(ToolView::Shell(ShellToolView {
            status: ShellViewStatus::Exited,
            command_id: CommandId::try_from(command).unwrap(),
            exit_code: Some(0),
            running_ms: 1,
            idle_ms: 0,
            chunks: Vec::new(),
            view_truncated: false,
            files_truncated: (!files.is_empty()).then_some(false),
            files: (!files.is_empty()).then_some(files),
            path_changes: changes,
            run_files: None,
            request_files: None,
        })),
    })
}

fn removed(path: &str) -> PathChange {
    PathChange::Removed {
        path: path.into(),
        temporary: path.starts_with("/tmp/"),
    }
}

fn renamed(from: &str, to: &str) -> PathChange {
    PathChange::Renamed {
        from: from.into(),
        to: to.into(),
        temporary: false,
    }
}

fn view(block: &Block) -> &ShellToolView {
    match block {
        Block::ToolCall(ToolCallBlock {
            view: Some(ToolView::Shell(view)),
            ..
        }) => view,
        other => panic!("not a shell call: {other:?}"),
    }
}

/// A list as a pill row reads it: each file's path, kind, old path and lines.
type Shown = (String, EditKind, Option<String>, Option<(u32, u32)>);

fn shown(files: &[ChangedFile]) -> Vec<Shown> {
    files
        .iter()
        .map(|file| {
            (
                file.path.clone(),
                file.kind,
                file.from.clone(),
                file.lines.map(|LineCounts { added, removed }| (added, removed)),
            )
        })
        .collect()
}

fn run_files(block: &Block) -> Vec<Shown> {
    shown(view(block).run_files.as_deref().expect("a shell call carries its run's files"))
}

fn request_files(block: &Block) -> Vec<Shown> {
    shown(view(block).request_files.as_deref().expect("a call of a request carries its files"))
}

/// A call's own pills: each file's path and kind.
fn own(block: &Block) -> Vec<(String, EditKind)> {
    view(block)
        .files
        .iter()
        .flatten()
        .map(|file| (file.path.clone(), file.kind))
        .collect()
}

fn entry(path: &str, kind: EditKind, from: Option<&str>, lines: Option<(u32, u32)>) -> Shown {
    (path.into(), kind, from.map(str::to_owned), lines)
}

#[test]
fn a_file_one_run_created_and_removed_shows_only_in_the_calls_that_did_it() {
    // shell1 creates `a`, shell4 removes it, in one run: opened, shell1
    // shows `a` and shell4 shows it struck; folded, the run shows neither,
    // and the request does not list it.
    let mut copies = Copies::default();
    let created = copies.edit("", "draft\n");
    let page = copies.edit("one\n", "one\ntwo\n");
    let mut blocks = vec![
        user("u"),
        shell("1", vec![written("/w/a.md", EditKind::Added, vec![created])], vec![]),
        shell("2", vec![written("/w/page.ts", EditKind::Modified, vec![page])], vec![]),
        shell("3", vec![], vec![]),
        shell("4", vec![], vec![removed("/w/a.md")]),
        text("done"),
    ];
    copies.list(&mut blocks);

    assert_eq!(own(&blocks[1]), [("/w/a.md".to_owned(), EditKind::Added)]);
    assert_eq!(own(&blocks[4]), [("/w/a.md".to_owned(), EditKind::Deleted)]);
    let page = entry("/w/page.ts", EditKind::Modified, None, Some((1, 0)));
    assert_eq!(run_files(&blocks[4]), [page.clone()]);
    assert_eq!(request_files(&blocks[4]), [page]);
    // Up to shell1, its run had made `a`.
    assert_eq!(run_files(&blocks[1]), [entry("/w/a.md", EditKind::Added, None, Some((1, 0)))]);
}

#[test]
fn a_file_created_in_one_run_and_removed_in_a_later_one_stays_in_the_first_runs_pills() {
    let mut copies = Copies::default();
    let created = copies.edit("", "draft\n");
    let mut blocks = vec![
        user("u"),
        shell("1", vec![written("/w/a.md", EditKind::Added, vec![created])], vec![]),
        text("Wrote the notes."),
        shell("2", vec![], vec![removed("/w/a.md")]),
        text("done"),
    ];
    copies.list(&mut blocks);

    assert_eq!(run_files(&blocks[1]), [entry("/w/a.md", EditKind::Added, None, Some((1, 0)))]);
    assert_eq!(run_files(&blocks[3]), [entry("/w/a.md", EditKind::Deleted, None, None)]);
    assert_eq!(request_files(&blocks[3]), []);
}

#[test]
fn a_file_that_was_there_before_and_was_removed_is_deleted_everywhere() {
    // `old.ts` was edited, then removed; `gone.ts` was only removed, so
    // nothing of it was kept to count.
    let mut copies = Copies::default();
    let edit = copies.edit("a\nb\nc\n", "a\nB\nc\n");
    let mut blocks = vec![
        user("u"),
        shell("1", vec![written("/w/old.ts", EditKind::Modified, vec![edit])], vec![]),
        shell("2", vec![], vec![removed("/w/old.ts"), removed("/w/gone.ts")]),
        text("done"),
    ];
    copies.list(&mut blocks);

    assert_eq!(
        own(&blocks[2]),
        [
            ("/w/old.ts".to_owned(), EditKind::Deleted),
            ("/w/gone.ts".to_owned(), EditKind::Deleted)
        ]
    );
    let deleted = [
        entry("/w/old.ts", EditKind::Deleted, None, Some((0, 3))),
        entry("/w/gone.ts", EditKind::Deleted, None, None),
    ];
    assert_eq!(run_files(&blocks[2]), deleted);
    assert_eq!(request_files(&blocks[2]), deleted);
}

#[test]
fn a_renamed_file_shows_under_its_new_name_with_its_edits() {
    // `c.ts` is edited, then moved, and `x.ts` only moved; the scratch file
    // in `/tmp` placed over `page.ts` shows as `page.ts`, modified.
    let mut copies = Copies::default();
    let edit = copies.edit("one\n", "one\ntwo\n");
    let scratch = copies.edit("", "new page\n");
    let replaced = copies.edit("old page\n", "new page\n");
    let mut blocks = vec![
        user("u"),
        shell(
            "1",
            vec![
                written("/w/c.ts", EditKind::Modified, vec![edit]),
                written("/tmp/page.new", EditKind::Added, vec![scratch]),
            ],
            vec![],
        ),
        shell(
            "2",
            vec![written("/w/page.ts", EditKind::Modified, vec![replaced])],
            vec![renamed("/w/c.ts", "/w/d.ts"), renamed("/w/x.ts", "/w/y.ts"), renamed("/tmp/page.new", "/w/page.ts")],
        ),
        text("done"),
    ];
    copies.list(&mut blocks);

    assert_eq!(
        own(&blocks[2]),
        [
            ("/w/page.ts".to_owned(), EditKind::Modified),
            ("/w/d.ts".to_owned(), EditKind::Renamed),
            ("/w/y.ts".to_owned(), EditKind::Renamed)
        ]
    );
    assert_eq!(own(&blocks[1]), [("/w/c.ts".to_owned(), EditKind::Modified), ("/tmp/page.new".to_owned(), EditKind::Added)]);
    let listed = [
        entry("/w/d.ts", EditKind::Renamed, Some("/w/c.ts"), Some((1, 0))),
        entry("/w/page.ts", EditKind::Modified, None, Some((1, 1))),
        entry("/w/y.ts", EditKind::Renamed, Some("/w/x.ts"), None),
    ];
    assert_eq!(request_files(&blocks[2]), listed);
    assert_eq!(run_files(&blocks[2]), listed);
    // The moved file keeps its edit, made under its old name.
    let request = view(&blocks[2]).request_files.as_ref().unwrap();
    assert_eq!(request[0].edits[0].path, "/w/c.ts");
    assert_eq!(request[1].edits.iter().map(|edit| edit.path.as_str()).collect::<Vec<_>>(), ["/tmp/page.new", "/w/page.ts"]);
}

#[test]
fn a_run_counts_a_file_from_before_the_run_and_a_request_from_before_the_request() {
    // A line two calls of one run changed counts once; the request's count
    // spans both runs.
    let mut copies = Copies::default();
    let first = copies.edit("a\n", "b\n");
    let second = copies.edit("b\n", "c\n");
    let third = copies.edit("c\n", "c\nd\n");
    let mut blocks = vec![
        user("u"),
        shell("1", vec![written("/w/f", EditKind::Modified, vec![first])], vec![]),
        shell("2", vec![written("/w/f", EditKind::Modified, vec![second])], vec![]),
        text("Next."),
        shell("3", vec![written("/w/f", EditKind::Modified, vec![third])], vec![]),
        text("done"),
    ];
    copies.list(&mut blocks);

    assert_eq!(run_files(&blocks[2]), [entry("/w/f", EditKind::Modified, None, Some((1, 1)))]);
    assert_eq!(run_files(&blocks[4]), [entry("/w/f", EditKind::Modified, None, Some((1, 0)))]);
    assert_eq!(request_files(&blocks[4]), [entry("/w/f", EditKind::Modified, None, Some((2, 1)))]);
}
