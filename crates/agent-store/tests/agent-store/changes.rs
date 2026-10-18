//! The lists of changed files a transcript's shell calls carry
//! (`edit-tracking.md` § What the conversation shows, § Within the working
//! directory): a call's own, its run's and its request's, as the session
//! lists a call within its agent's directory and a stored transcript is
//! listed again in order.

use std::collections::HashMap;

use bytes::Bytes;
use demi_agent_store::{
    changes::{count_ends, plan, relist, set_runs},
    testing::test_model,
};
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
            binary: false,
        }
    }

    /// Lists `blocks` as a stored transcript is listed again, where the
    /// directory is not known.
    fn list(&self, blocks: &mut [Block]) {
        relist(blocks, |blob| Ok::<_, ()>(self.0.get(blob).cloned())).unwrap();
    }

    /// Lists each shell call of `blocks` in order as the session does when
    /// it stores the call, its agent working in `directory`.
    fn list_in(&self, blocks: &mut [Block], directory: &str) {
        self.list_started_in(blocks, |_| directory.to_owned());
    }

    /// [`list_in`](Self::list_in), each call started in the directory
    /// `started_in` gives for its command.
    fn list_started_in(&self, blocks: &mut [Block], started_in: impl Fn(&str) -> String) {
        set_runs(blocks);
        for index in 0..blocks.len() {
            let Block::ToolCall(ToolCallBlock {
                view: Some(ToolView::Shell(view)),
                ..
            }) = &blocks[index]
            else {
                continue;
            };
            let mut view = view.clone();
            let directory = started_in(view.command_id.as_str());
            let planned = plan(blocks, index, &view, Some(&directory));
            let counted = planned
                .uncounted()
                .into_iter()
                .map(|(original, modified)| {
                    let lines = count_ends(&original, modified.as_ref(), |blob| self.0.get(blob).cloned());
                    ((original, modified), lines)
                })
                .collect();
            planned.finish(&mut view, &counted);
            if let Block::ToolCall(call) = &mut blocks[index] {
                call.view = Some(ToolView::Shell(view));
            }
        }
    }
}

fn id(value: &str) -> BlockId {
    BlockId::try_from(value).unwrap()
}

fn user(value: &str) -> Block {
    Block::User(UserBlock {
        comments: Vec::new(),
        entries: Vec::new(),
        id: id(value),
        turn_id: TurnId::try_from(value).unwrap(),
        created_at: Timestamp::UNIX_EPOCH,
        model: test_model(),
        content: Vec::new(),
        preamble: None,
        automation: None,
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

/// A file a job wrote, as a transcript stored before the lists were made
/// marks it: in `/tmp`, outside its agent's directory.
fn written(path: &str, kind: EditKind, edits: Vec<EditSegment>) -> EditedFile {
    EditedFile {
        path: path.into(),
        kind,
        from: None,
        added: 0,
        removed: 0,
        outside: path.starts_with("/tmp/"),
        edits,
    }
}

/// A shell call whose command `command` wrote `files` and made `changes`.
fn shell(command: &str, files: Vec<EditedFile>, changes: Vec<PathChange>) -> Block {
    Block::ToolCall(ToolCallBlock {
        run: None,
        entries: Vec::new(),
        ended_at: None,
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
            lost_reason: None,
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
    PathChange::Removed { path: path.into() }
}

fn renamed(from: &str, to: &str) -> PathChange {
    PathChange::Renamed {
        from: from.into(),
        to: to.into(),
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

/// A call's own record: each file's path and kind.
fn own(block: &Block) -> Vec<(String, EditKind)> {
    view(block)
        .files
        .iter()
        .flatten()
        .map(|file| (file.path.clone(), file.kind))
        .collect()
}

/// A call's own pills: the paths of its record not marked outside.
fn pills(block: &Block) -> Vec<String> {
    view(block)
        .files
        .iter()
        .flatten()
        .filter(|file| !file.outside)
        .map(|file| file.path.clone())
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

#[test]
fn only_files_within_the_agents_directory_show_and_a_move_into_it_brings_its_edits() {
    // The agent works in `/home/u/shop`. Its first call writes a scratch
    // file in `/tmp`, edits `~/.zshrc`, writes in the conversation's scratch
    // directory under `~/.demi` and in a sibling checkout whose name
    // begins with the directory's, and edits `src/cart.ts`: only
    // `src/cart.ts` shows. Its second moves the scratch file over
    // `src/page.ts`, which then shows with the edit made in `/tmp`.
    let mut copies = Copies::default();
    let fresh = |copies: &mut Copies, text: &str| copies.edit("", text);
    let cart = copies.edit("a\n", "a\nb\n");
    let zshrc = copies.edit("x\n", "y\n");
    let draft = fresh(&mut copies, "new page\n");
    let notes = fresh(&mut copies, "notes\n");
    let sibling = fresh(&mut copies, "other\n");
    let unmarked = |path: &str, kind, edits| EditedFile {
        outside: false,
        ..written(path, kind, edits)
    };
    let mut blocks = vec![
        user("u"),
        shell(
            "1",
            vec![
                unmarked("/tmp/page.ts.new", EditKind::Added, vec![draft]),
                unmarked("/home/u/.zshrc", EditKind::Modified, vec![zshrc]),
                unmarked("/home/u/.demi/scratch/c/notes.md", EditKind::Added, vec![notes]),
                unmarked("/home/u/shop-old/x.ts", EditKind::Added, vec![sibling]),
                unmarked("/home/u/shop/src/cart.ts", EditKind::Modified, vec![cart]),
            ],
            vec![],
        ),
        shell("2", vec![], vec![renamed("/tmp/page.ts.new", "/home/u/shop/src/page.ts")]),
        text("done"),
    ];
    copies.list_in(&mut blocks, "/home/u/shop/");

    let cart = entry("/home/u/shop/src/cart.ts", EditKind::Modified, None, Some((1, 0)));
    assert_eq!(pills(&blocks[1]), ["/home/u/shop/src/cart.ts"]);
    assert_eq!(own(&blocks[1]).len(), 5, "the record keeps every file");
    assert_eq!(run_files(&blocks[1]), [cart.clone()]);
    assert_eq!(request_files(&blocks[1]), [cart.clone()]);
    let page = entry("/home/u/shop/src/page.ts", EditKind::Added, None, Some((1, 0)));
    assert_eq!(pills(&blocks[2]), ["/home/u/shop/src/page.ts"]);
    assert_eq!(run_files(&blocks[2]), [page.clone(), cart.clone()]);
    assert_eq!(request_files(&blocks[2]), [page, cart]);
}

#[test]
fn a_path_out_through_dot_dot_or_in_windows_spelling_is_judged_by_its_parts() {
    // `shop/../.zshrc` is `~/.zshrc`, outside; on Windows a path in either
    // separator and case is inside, one out through `..` is not.
    let mut copies = Copies::default();
    let mut edit = |text: &str| copies.edit("", text);
    let (zshrc, cart) = (edit("x\n"), edit("y\n"));
    let (windows, gitconfig) = (edit("w\n"), edit("g\n"));
    let unmarked = |path: &str, edits| EditedFile {
        outside: false,
        ..written(path, EditKind::Added, edits)
    };
    let mut posix = vec![
        user("u"),
        shell(
            "1",
            vec![
                unmarked("/home/u/shop/../.zshrc", vec![zshrc]),
                unmarked("/home/u/shop/./src/cart.ts", vec![cart]),
            ],
            vec![],
        ),
        text("done"),
    ];
    copies.list_in(&mut posix, "/home/u/shop");
    assert_eq!(pills(&posix[1]), ["/home/u/shop/./src/cart.ts"]);
    assert_eq!(
        request_files(&posix[1]).into_iter().map(|file| file.0).collect::<Vec<_>>(),
        ["/home/u/shop/./src/cart.ts"],
        "the record keeps the path as the job wrote it"
    );

    let mut windows = vec![
        user("u"),
        shell(
            "1",
            vec![
                unmarked("c:/users/U/SHOP/src/cart.ts", vec![windows]),
                unmarked(r"C:\Users\u\shop\..\.gitconfig", vec![gitconfig]),
            ],
            vec![],
        ),
        text("done"),
    ];
    copies.list_in(&mut windows, r"C:\Users\u\shop");
    assert_eq!(pills(&windows[1]), ["c:/users/U/SHOP/src/cart.ts"]);
}

#[test]
fn a_calls_files_are_judged_against_the_directory_it_started_in_and_never_again() {
    // The first call starts in `shop` and writes `src/a.ts` there; then the
    // agent's directory becomes `shop/docs`, where the second call writes
    // `guide.md`. The second call's lists keep `src/a.ts`, judged as the
    // first call was stored, beside `guide.md`; its own `src/b.ts`, outside
    // `shop/docs`, is left out.
    let mut copies = Copies::default();
    let (a, guide, b) = (copies.edit("", "a\n"), copies.edit("", "g\n"), copies.edit("", "b\n"));
    let mut blocks = vec![
        user("u"),
        shell("1", vec![written("/home/u/shop/src/a.ts", EditKind::Added, vec![a])], vec![]),
        shell(
            "2",
            vec![
                written("/home/u/shop/docs/guide.md", EditKind::Added, vec![guide]),
                written("/home/u/shop/src/b.ts", EditKind::Added, vec![b]),
            ],
            vec![],
        ),
        text("done"),
    ];
    copies.list_started_in(&mut blocks, |command| {
        if command == "1" { "/home/u/shop" } else { "/home/u/shop/docs" }.to_owned()
    });

    assert_eq!(pills(&blocks[1]), ["/home/u/shop/src/a.ts"]);
    assert_eq!(pills(&blocks[2]), ["/home/u/shop/docs/guide.md"]);
    let paths = |files: Vec<Shown>| files.into_iter().map(|file| file.0).collect::<Vec<_>>();
    assert_eq!(paths(request_files(&blocks[2])), ["/home/u/shop/src/a.ts", "/home/u/shop/docs/guide.md"]);
    assert_eq!(paths(run_files(&blocks[2])), ["/home/u/shop/src/a.ts", "/home/u/shop/docs/guide.md"]);
}
