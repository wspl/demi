use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use demi_runner_host::git::{Changes, GitError, GitService};
use demi_runner_protocol::wire::{ChangeKind, Inbound};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

fn git(repo: &Path, args: &[&str]) {
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .status()
        .expect("git on PATH");
    assert!(status.success(), "git {args:?} failed");
}

/// A repository with one commit: `a.txt` (three lines), `b.txt` (two lines)
/// and `dir/c.txt` (one line).
fn committed_repo() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(dir.path()).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "1\n2\n3\n").unwrap();
    std::fs::write(repo.join("b.txt"), "x\ny\n").unwrap();
    std::fs::create_dir(repo.join("dir")).unwrap();
    std::fs::write(repo.join("dir/c.txt"), "c\n").unwrap();
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "first"]);
    (dir, repo)
}

async fn changes(service: &GitService, root: &Path) -> Changes {
    service
        .changes(root, &CancellationToken::new())
        .await
        .unwrap()
}

type Summary = (String, String, ChangeKind, Option<String>, u64, u64);

fn summary(changes: &Changes) -> Vec<Summary> {
    changes
        .files
        .iter()
        .map(|file| {
            (
                file.path.clone(),
                file.status.clone(),
                file.kind,
                file.from.clone(),
                file.added,
                file.removed,
            )
        })
        .collect()
}

#[tokio::test]
async fn a_directory_outside_any_repository_is_not_a_repository() {
    let dir = tempfile::tempdir().unwrap();
    let service = GitService::default();
    let result = changes(&service, dir.path()).await;
    assert!(!result.repository);
    assert!(result.files.is_empty());
    assert!(!result.watched);
    let error = service
        .show(dir.path(), "a.txt", None, &CancellationToken::new())
        .await
        .unwrap_err();
    assert_eq!(error.code(), "not_repository");
}

#[tokio::test]
async fn changes_are_judged_against_head_with_line_counts() {
    let (_dir, repo) = committed_repo();
    std::fs::write(repo.join("a.txt"), "1\n2 changed\n3\n4\n").unwrap();
    std::fs::remove_file(repo.join("b.txt")).unwrap();
    std::fs::write(repo.join("d.txt"), "new\nfile").unwrap();
    git(&repo, &["mv", "dir/c.txt", "dir/e.txt"]);
    // Staged and then reverted on disk: nothing against HEAD.
    std::fs::write(repo.join("a2.txt"), "staged\n").unwrap();
    git(&repo, &["add", "a2.txt"]);
    std::fs::remove_file(repo.join("a2.txt")).unwrap();

    let service = GitService::default();
    let result = changes(&service, &repo).await;
    assert!(result.repository);
    assert!(result.head.is_some());
    assert!(!result.truncated);
    assert_eq!(
        summary(&result),
        vec![
            (
                "a.txt".into(),
                " M".into(),
                ChangeKind::Modified,
                None,
                2,
                1
            ),
            ("b.txt".into(), " D".into(), ChangeKind::Deleted, None, 0, 2),
            ("d.txt".into(), "??".into(), ChangeKind::Added, None, 2, 0),
            (
                "dir/e.txt".into(),
                "R ".into(),
                ChangeKind::Renamed,
                Some("dir/c.txt".into()),
                0,
                0
            ),
        ]
    );
}

/// Runs git where a failure is the point, as a merge that stops on a conflict.
fn git_may_fail(repo: &Path, args: &[&str]) {
    Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .output()
        .expect("git on PATH");
}

/// What `git status --porcelain -z --untracked-files=all` prints: each path
/// with its two letters, a rename or copy at its new path. It writes
/// nothing: git would otherwise refresh the index, a change under `.git`.
fn git_status(repo: &Path) -> BTreeMap<String, String> {
    let output = Command::new("git")
        .args([
            "--no-optional-locks",
            "status",
            "--porcelain=v1",
            "-z",
            "--untracked-files=all",
        ])
        .current_dir(repo)
        .output()
        .expect("git on PATH");
    assert!(output.status.success());
    let text = String::from_utf8(output.stdout).unwrap();
    let mut fields = text.split('\0').filter(|field| !field.is_empty());
    let mut statuses = BTreeMap::new();
    while let Some(field) = fields.next() {
        let (letters, path) = field.split_at(2);
        // A rename's or a copy's old path follows its new one.
        if letters.contains('R') || letters.contains('C') {
            fields.next();
        }
        statuses.insert(path[1..].to_owned(), letters.to_owned());
    }
    statuses
}

/// Each listed path with its two letters, to compare with `git_status`.
fn statuses(changes: &Changes) -> BTreeMap<String, String> {
    changes
        .files
        .iter()
        .map(|file| (file.path.clone(), file.status.clone()))
        .collect()
}

#[tokio::test]
async fn every_path_carries_the_letters_git_status_prints_for_it() {
    let (_dir, repo) = committed_repo();
    for name in [
        "s.txt",
        "d.txt",
        "e.txt",
        "f.txt",
        "t.txt",
        "x.sh",
        "clash.txt",
    ] {
        std::fs::write(repo.join(name), format!("{name}\n")).unwrap();
    }
    git(&repo, &["add", "."]);
    git(&repo, &["commit", "-q", "-m", "more"]);
    // A conflict first: the merge needs the files it touches clean.
    git(&repo, &["checkout", "-q", "-b", "other"]);
    std::fs::write(repo.join("clash.txt"), "theirs\n").unwrap();
    git(&repo, &["commit", "-q", "-am", "theirs"]);
    git(&repo, &["checkout", "-q", "main"]);
    std::fs::write(repo.join("clash.txt"), "ours\n").unwrap();
    git(&repo, &["commit", "-q", "-am", "ours"]);
    git_may_fail(&repo, &["merge", "-q", "other"]);

    // Edited, not staged; staged; staged and edited again.
    std::fs::write(repo.join("a.txt"), "1\n2\n3\n4\n").unwrap();
    std::fs::write(repo.join("b.txt"), "x\ny\nz\n").unwrap();
    git(&repo, &["add", "b.txt"]);
    std::fs::write(repo.join("s.txt"), "staged\n").unwrap();
    git(&repo, &["add", "s.txt"]);
    std::fs::write(repo.join("s.txt"), "staged, then edited\n").unwrap();
    // Deleted, not staged; deleted and staged.
    std::fs::remove_file(repo.join("d.txt")).unwrap();
    git(&repo, &["rm", "-q", "e.txt"]);
    // New: untracked; staged; staged and edited; added with intent.
    std::fs::write(repo.join("untracked.txt"), "u\n").unwrap();
    std::fs::write(repo.join("added.txt"), "a\n").unwrap();
    git(&repo, &["add", "added.txt"]);
    std::fs::write(repo.join("am.txt"), "a\n").unwrap();
    git(&repo, &["add", "am.txt"]);
    std::fs::write(repo.join("am.txt"), "a\nm\n").unwrap();
    std::fs::write(repo.join("intent.txt"), "i\n").unwrap();
    git(&repo, &["add", "-N", "intent.txt"]);
    // Renamed and staged; moved without staging.
    git(&repo, &["mv", "dir/c.txt", "dir/moved.txt"]);
    std::fs::rename(repo.join("f.txt"), repo.join("g.txt")).unwrap();
    // Only the executable bit; a file become a symlink.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(repo.join("x.sh"), std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::remove_file(repo.join("t.txt")).unwrap();
    std::os::unix::fs::symlink("a.txt", repo.join("t.txt")).unwrap();

    let result = changes(&GitService::default(), &repo).await;
    assert_eq!(statuses(&result), git_status(&repo));
    // A change git lists whose bytes match HEAD counts no lines.
    let executable = result
        .files
        .iter()
        .find(|file| file.path == "x.sh")
        .unwrap();
    assert_eq!(
        (executable.kind, executable.added, executable.removed),
        (ChangeKind::Modified, 0, 0)
    );
}

#[tokio::test]
async fn a_root_inside_the_work_tree_lists_its_subtree_with_relative_paths() {
    let (_dir, repo) = committed_repo();
    std::fs::write(repo.join("a.txt"), "changed\n").unwrap();
    std::fs::write(repo.join("dir/new.txt"), "n\n").unwrap();
    git(&repo, &["mv", "dir/c.txt", "dir/e.txt"]);

    let service = GitService::default();
    let result = changes(&service, &repo.join("dir")).await;
    assert_eq!(
        summary(&result),
        vec![
            (
                "e.txt".into(),
                "R ".into(),
                ChangeKind::Renamed,
                Some("c.txt".into()),
                0,
                0
            ),
            ("new.txt".into(), "??".into(), ChangeKind::Added, None, 1, 0),
        ]
    );
    let shown = service
        .show(&repo.join("dir"), "c.txt", None, &CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(shown, b"c\n");
}

#[tokio::test]
async fn the_list_stops_at_the_file_limit() {
    let (_dir, repo) = committed_repo();
    for name in ["n1", "n2", "n3"] {
        std::fs::write(repo.join(name), "x\n").unwrap();
    }
    let service = GitService::with_limits(2);
    let result = changes(&service, &repo).await;
    assert_eq!(result.files.len(), 2);
    assert!(result.truncated);
}

#[tokio::test]
async fn show_reads_the_last_commit_and_refuses_what_it_lacks() {
    let (_dir, repo) = committed_repo();
    std::fs::write(repo.join("a.txt"), "changed\n").unwrap();
    std::fs::write(repo.join("untracked.txt"), "u\n").unwrap();
    let service = GitService::default();
    let cancel = CancellationToken::new();
    assert_eq!(
        service.show(&repo, "a.txt", None, &cancel).await.unwrap(),
        b"1\n2\n3\n"
    );
    let missing = service
        .show(&repo, "untracked.txt", None, &cancel)
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "ENOENT");
    let directory = service.show(&repo, "dir", None, &cancel).await.unwrap_err();
    assert_eq!(directory.code(), "EISDIR");
}

#[tokio::test]
async fn a_cancelled_request_answers_cancelled() {
    let (_dir, repo) = committed_repo();
    let service = GitService::default();
    let cancel = CancellationToken::new();
    cancel.cancel();
    let error = service.changes(&repo, &cancel).await.unwrap_err();
    assert!(matches!(error, GitError::Cancelled));
}

/// Polls the watched baseline until `settled` holds for it, or fails after five seconds.
async fn poll(service: &GitService, root: &Path, settled: impl Fn(&Changes) -> bool) -> Changes {
    poll_within(
        service,
        root,
        Duration::from_secs(5),
        "the watched baseline never settled",
        settled,
    )
    .await
}

/// Polls until the root's watch runs, which no request waits for. FSEvents
/// takes seconds to start a stream while builds flood fseventsd, and once
/// took more than 30 in the main checkout.
async fn watching(service: &GitService, root: &Path) -> Changes {
    poll_within(
        service,
        root,
        Duration::from_secs(60),
        "the watch never started",
        |result| result.watched,
    )
    .await
}

async fn poll_within(
    service: &GitService,
    root: &Path,
    within: Duration,
    failure: &str,
    settled: impl Fn(&Changes) -> bool,
) -> Changes {
    let deadline = Instant::now() + within;
    loop {
        let result = changes(service, root).await;
        if settled(&result) {
            return result;
        }
        assert!(
            Instant::now() < deadline,
            "{failure}; last {:?}",
            summary(&result)
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Polls until the watched baseline lists exactly `expected`, or fails after five seconds.
async fn wait_for(service: &GitService, root: &Path, expected: &[(&str, ChangeKind)]) -> Changes {
    poll(service, root, |result| {
        result
            .files
            .iter()
            .map(|file| (file.path.as_str(), file.kind))
            .eq(expected.iter().copied())
    })
    .await
}

#[tokio::test]
async fn later_requests_follow_the_watch_and_a_commit_starts_over() {
    let (_dir, repo) = committed_repo();
    let service = GitService::default();
    // The first request starts the watch without waiting for it; a later one
    // answers from it once it runs.
    let first = watching(&service, &repo).await;
    assert!(first.files.is_empty());

    std::fs::write(repo.join("a.txt"), "1\n2\n3\nmore\n").unwrap();
    let modified = wait_for(&service, &repo, &[("a.txt", ChangeKind::Modified)]).await;
    assert_eq!(modified.files[0].added, 1);

    std::fs::write(repo.join("dir/new.txt"), "n\n").unwrap();
    std::fs::remove_file(repo.join("b.txt")).unwrap();
    wait_for(
        &service,
        &repo,
        &[
            ("a.txt", ChangeKind::Modified),
            ("b.txt", ChangeKind::Deleted),
            ("dir/new.txt", ChangeKind::Added),
        ],
    )
    .await;

    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "second"]);
    let committed = wait_for(&service, &repo, &[]).await;
    assert_ne!(committed.head, first.head);
    assert!(committed.watched);
}

#[tokio::test]
async fn watched_requests_follow_ignore_rules_and_modes() {
    let (_dir, repo) = committed_repo();
    std::fs::write(repo.join("dir/new.txt"), "n\n").unwrap();
    std::fs::write(repo.join("scratch.log"), "l\n").unwrap();
    let service = GitService::default();
    let first = changes(&service, &repo).await;
    assert_eq!(statuses(&first), git_status(&repo));

    // A rule that ignores a listed file takes it off the list.
    std::fs::write(repo.join(".gitignore"), "*.log\n").unwrap();
    let ignored = poll(&service, &repo, |result| {
        !statuses(result).contains_key("scratch.log")
    })
    .await;
    assert_eq!(statuses(&ignored), git_status(&repo));

    // Only the mode changes.
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(repo.join("b.txt"), std::fs::Permissions::from_mode(0o755)).unwrap();
    let executable = poll(&service, &repo, |result| {
        statuses(result).contains_key("b.txt")
    })
    .await;
    assert_eq!(statuses(&executable), git_status(&repo));
}

#[tokio::test]
async fn a_rule_above_the_root_reaches_under_it() {
    let (_dir, repo) = committed_repo();
    std::fs::write(repo.join("dir/new.log"), "n\n").unwrap();
    let root = repo.join("dir");
    let service = GitService::default();
    let first = changes(&service, &root).await;
    assert_eq!(
        statuses(&first).into_keys().collect::<Vec<_>>(),
        ["new.log"]
    );

    std::fs::write(repo.join(".gitignore"), "*.log\n").unwrap();
    poll(&service, &root, |result| result.files.is_empty()).await;
}

#[tokio::test]
async fn a_staged_rename_stays_one_entry_across_watched_requests() {
    let (_dir, repo) = committed_repo();
    git(&repo, &["mv", "a.txt", "moved.txt"]);
    let service = GitService::default();
    let first = watching(&service, &repo).await;
    assert_eq!(statuses(&first), git_status(&repo));

    // Only the new path changes.
    std::fs::write(repo.join("moved.txt"), "1\n2\n3\n4\n").unwrap();
    let edited = poll(&service, &repo, |result| {
        statuses(result).get("moved.txt").map(String::as_str) != Some("R ")
    })
    .await;
    assert_eq!(
        summary(&edited),
        vec![(
            "moved.txt".into(),
            "RM".into(),
            ChangeKind::Renamed,
            Some("a.txt".into()),
            1,
            0
        )]
    );
    assert_eq!(statuses(&edited), git_status(&repo));

    // Only the old path changes: a file comes and goes there.
    std::fs::write(repo.join("a.txt"), "back\n").unwrap();
    std::fs::remove_file(repo.join("a.txt")).unwrap();
    std::fs::write(repo.join("b.txt"), "x\ny\nz\n").unwrap();
    let touched = poll(&service, &repo, |result| {
        statuses(result).contains_key("b.txt")
    })
    .await;
    assert_eq!(statuses(&touched), git_status(&repo));
}

/// A computation that fails has consumed what the watch recorded; the next
/// request still lists those changes.
#[tokio::test]
async fn a_failed_computation_leaves_its_changes_to_the_next_request() {
    // The clone keeps its objects in the store, outside the watched trees,
    // so taking a blob away and back changes nothing the watch sees.
    let (_store_dir, store) = committed_repo();
    let clone_dir = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(clone_dir.path()).unwrap();
    let clone = ["clone", "-q", "--shared", store.to_str().unwrap(), repo.to_str().unwrap()];
    git(&store, &clone);
    let blob = Command::new("git")
        .args(["rev-parse", "HEAD:a.txt"])
        .current_dir(&repo)
        .output()
        .unwrap();
    let blob = String::from_utf8(blob.stdout).unwrap().trim().to_owned();
    let object = store.join(".git/objects").join(&blob[..2]).join(&blob[2..]);
    let aside = store.join("blob-aside");
    let service = GitService::default();
    watching(&service, &repo).await;

    // The request that examines the edit cannot read the last commit's
    // copy of the file; those before it answer from the baseline.
    std::fs::rename(&object, &aside).unwrap();
    std::fs::write(repo.join("a.txt"), "1\n2\n3\nmore\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match service.changes(&repo, &CancellationToken::new()).await {
            Err(error) => {
                assert_eq!(error.code(), "internal", "{error}");
                break;
            }
            Ok(result) => {
                assert!(result.files.is_empty(), "{:?}", summary(&result));
                assert!(Instant::now() < deadline, "the edit never failed a request");
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
    }

    std::fs::rename(&aside, &object).unwrap();
    assert_eq!(
        summary(&changes(&service, &repo).await),
        vec![(
            "a.txt".into(),
            " M".into(),
            ChangeKind::Modified,
            None,
            1,
            0
        )]
    );
}

#[derive(Deserialize)]
struct Reply {
    #[serde(rename = "type")]
    kind: String,
    result: Option<serde_json::Value>,
    code: Option<String>,
}

async fn call(service: &GitService, message: Inbound, cwd: &Path) -> Reply {
    let bytes = demi_runner_host::git::handle(service, &message, cwd, &CancellationToken::new())
        .await
        .unwrap()
        .unwrap()
        .into_bytes();
    rmp_serde::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn the_wire_carries_changes_and_errors() {
    let (_dir, repo) = committed_repo();
    std::fs::write(repo.join("d.txt"), "new\n").unwrap();
    let service = GitService::default();
    let reply = call(
        &service,
        Inbound::GitChanges {
            id: "1".into(),
            root: ".".into(),
        },
        &repo,
    )
    .await;
    assert_eq!(reply.kind, "git_ok");
    let result = reply.result.unwrap();
    assert_eq!(result["repository"], true);
    assert_eq!(result["files"][0]["path"], "d.txt");
    assert_eq!(result["files"][0]["status"], "??");
    assert_eq!(result["files"][0]["kind"], "added");
    assert_eq!(result["files"][0]["added"], 1);

    let reply = call(
        &service,
        Inbound::GitChanges {
            id: "2".into(),
            root: "missing".into(),
        },
        &repo,
    )
    .await;
    assert_eq!(reply.kind, "git_error");
    assert_eq!(reply.code.as_deref(), Some("ENOENT"));
}

/// Runs git with both of a commit's times at `seconds`, so the order of
/// branches by their last commit is the test's to decide.
fn git_at(repo: &Path, seconds: i64, args: &[&str]) {
    let date = format!("@{seconds} +0000");
    let status = Command::new("git")
        .args(args)
        .current_dir(repo)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_DATE", &date)
        .status()
        .expect("git on PATH");
    assert!(status.success(), "git {args:?} failed");
}

fn git_output(repo: &Path, args: &[&str]) -> String {
    let output = Command::new("git").args(args).current_dir(repo).output().expect("git on PATH");
    assert!(output.status.success(), "git {args:?} failed");
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

/// A history to read: on `main`, a root commit adding `a.txt` and `b.txt`,
/// 52 commits each appending to `a.txt`, then a merge of `side`, which
/// renamed `b.txt` and added `side.txt`; `old` stops at the root commit;
/// `main` is checked out. Each commit is a second later than the one before.
struct History {
    _dir: tempfile::TempDir,
    repo: PathBuf,
    root_commit: String,
    side_commit: String,
    merge: String,
}

fn history() -> History {
    let dir = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(dir.path()).unwrap();
    let mut time = 1_700_000_000;
    let mut tick = || {
        time += 1;
        time
    };
    git(&repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("a.txt"), "0\n").unwrap();
    std::fs::write(repo.join("b.txt"), "b1\nb2\nb3\n").unwrap();
    git(&repo, &["add", "."]);
    git_at(&repo, tick(), &["commit", "-q", "-m", "Start\n\nThe first commit."]);
    let root_commit = git_output(&repo, &["rev-parse", "HEAD"]);
    git(&repo, &["branch", "old"]);
    git_at(&repo, tick(), &["checkout", "-q", "-b", "side"]);
    git(&repo, &["mv", "b.txt", "c.txt"]);
    std::fs::write(repo.join("side.txt"), "s1\ns2\n").unwrap();
    git(&repo, &["add", "."]);
    git_at(&repo, tick(), &["commit", "-q", "-m", "Side work"]);
    let side_commit = git_output(&repo, &["rev-parse", "HEAD"]);
    git(&repo, &["checkout", "-q", "main"]);
    let mut lines = String::from("0\n");
    for number in 1..=52 {
        lines.push_str(&format!("{number}\n"));
        std::fs::write(repo.join("a.txt"), &lines).unwrap();
        git_at(&repo, tick(), &["commit", "-q", "-am", &format!("Line {number}")]);
    }
    git_at(&repo, tick(), &["merge", "-q", "--no-ff", "-m", "Merge side", "side"]);
    let merge = git_output(&repo, &["rev-parse", "HEAD"]);
    History {
        _dir: dir,
        repo,
        root_commit,
        side_commit,
        merge,
    }
}

// About two seconds: the history takes some sixty git commands.
#[tokio::test]
async fn the_log_pages_a_branchs_first_parents_and_a_commit_shows_its_files_and_branches() {
    let history = history();
    let repo = &history.repo;
    let service = GitService::default();
    let cancel = CancellationToken::new();

    // HEAD's history by first parents: the merge, 52 lines, the root; not
    // the side branch's commit.
    let first = service.log(repo, None, None, &cancel).await.unwrap();
    assert_eq!(first.commits.len(), 50);
    assert!(first.more);
    assert_eq!(first.commits[0].id, history.merge);
    assert_eq!(first.commits[0].message.trim(), "Merge side");
    assert_eq!(first.commits[0].author, "Test");
    assert_eq!(first.commits[1].message.trim(), "Line 52");
    let rest = service
        .log(repo, None, Some(&first.commits[49].id), &cancel)
        .await
        .unwrap();
    assert!(!rest.more);
    let messages: Vec<&str> = rest.commits.iter().map(|commit| commit.message.trim()).collect();
    assert_eq!(messages, ["Line 3", "Line 2", "Line 1", "Start\n\nThe first commit."]);
    assert_eq!(rest.commits[3].id, history.root_commit);
    assert!(first.commits.iter().chain(&rest.commits).all(|commit| commit.id != history.side_commit));
    // Times are the commits', newest first.
    assert!(first.commits[0].time > first.commits[1].time);
    // Another branch's history, by name.
    let side = service.log(repo, Some("side"), None, &cancel).await.unwrap();
    assert_eq!(
        side.commits.iter().map(|commit| commit.id.as_str()).collect::<Vec<_>>(),
        [history.side_commit.as_str(), history.root_commit.as_str()]
    );
    assert!(!side.more);

    // The merge against its first parent: what the side branch brought.
    let merge = service.commit(repo, &history.merge, &cancel).await.unwrap();
    assert_eq!(merge.commit.parents.len(), 2);
    assert_eq!(merge.commit.parents[1], history.side_commit);
    let files: Vec<_> = merge
        .files
        .iter()
        .map(|file| (file.path.as_str(), file.status.as_str(), file.kind, file.from.as_deref(), file.added, file.removed))
        .collect();
    assert_eq!(
        files,
        [
            ("c.txt", "R", ChangeKind::Renamed, Some("b.txt"), 0, 0),
            ("side.txt", "A", ChangeKind::Added, None, 2, 0),
        ]
    );
    assert_eq!(merge.branches, ["main"]);
    // A root commit adds everything; it is on every branch, the checked-out one first.
    let root = service
        .commit(repo, &history.root_commit[..7], &cancel)
        .await
        .unwrap();
    assert_eq!(root.commit.id, history.root_commit);
    assert!(root.commit.parents.is_empty());
    let files: Vec<_> = root.files.iter().map(|file| (file.path.as_str(), file.status.as_str(), file.added)).collect();
    assert_eq!(files, [("a.txt", "A", 1), ("b.txt", "A", 3)]);
    assert_eq!(root.branches, ["main", "side", "old"]);
    let line = &first.commits[1];
    let modified = service.commit(repo, &line.id, &cancel).await.unwrap();
    assert_eq!(
        modified.files.iter().map(|file| (file.path.as_str(), file.status.as_str(), file.added, file.removed)).collect::<Vec<_>>(),
        [("a.txt", "M", 1, 0)]
    );
    assert_eq!(service.commit(repo, &history.side_commit, &cancel).await.unwrap().branches, ["main", "side"]);

    // A file as a commit has it.
    let start = service
        .show(repo, "a.txt", Some(&history.root_commit), &cancel)
        .await
        .unwrap();
    assert_eq!(start, b"0\n");
    let gone = service
        .show(repo, "c.txt", Some(&history.root_commit), &cancel)
        .await
        .unwrap_err();
    assert_eq!(gone.code(), "ENOENT");

    // An id the repository lacks, and a branch that no longer exists.
    let missing = service.commit(repo, "0000000", &cancel).await.unwrap_err();
    assert_eq!(missing.code(), "ENOENT");
    git(repo, &["branch", "-q", "-D", "old"]);
    let deleted = service.log(repo, Some("old"), None, &cancel).await.unwrap_err();
    assert_eq!(deleted.code(), "ENOENT");
}

// Under a second: a few git commands and some ten thousand hashes.
#[tokio::test]
async fn branches_order_by_their_last_commit_and_a_shared_abbreviation_is_ambiguous() {
    let dir = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(dir.path()).unwrap();
    let service = GitService::default();
    let cancel = CancellationToken::new();
    git(&repo, &["init", "-q", "-b", "main"]);
    // No commit yet: the branch checked out is named, and none is listed.
    let unborn = service.branches(&repo, &cancel).await.unwrap();
    assert_eq!((unborn.branches.len(), unborn.current.as_deref(), unborn.head), (0, Some("main"), None));
    assert!(service.log(&repo, None, None, &cancel).await.unwrap().commits.is_empty());

    std::fs::write(repo.join("a.txt"), "a\n").unwrap();
    git(&repo, &["add", "."]);
    git_at(&repo, 1_700_000_000, &["commit", "-q", "-m", "first"]);
    git(&repo, &["checkout", "-q", "-b", "newer"]);
    git_at(&repo, 1_700_000_200, &["commit", "-q", "--allow-empty", "-m", "newer"]);
    git(&repo, &["checkout", "-q", "-b", "middle", "main"]);
    git_at(&repo, 1_700_000_100, &["commit", "-q", "--allow-empty", "-m", "middle"]);
    let branches = service.branches(&repo, &cancel).await.unwrap();
    assert_eq!(branches.branches, ["newer", "middle", "main"]);
    assert_eq!((branches.current.as_deref(), branches.head), (Some("middle"), None));

    // A detached HEAD names its commit instead of a branch.
    let main = git_output(&repo, &["rev-parse", "main"]);
    git(&repo, &["checkout", "-q", "--detach", "main"]);
    let detached = service.branches(&repo, &cancel).await.unwrap();
    assert_eq!((detached.current, detached.head), (None, Some(main.clone())));

    // Two commits whose ids share their first seven hex digits.
    let tree = git_output(&repo, &["rev-parse", "main^{tree}"]);
    let mut seen = std::collections::HashMap::new();
    let (one, two) = (0u64..)
        .find_map(|number| {
            let body = format!(
                "tree {tree}\nauthor Test <test@example.com> 1700000000 +0000\ncommitter Test <test@example.com> 1700000000 +0000\n\n{number}\n"
            );
            let id = gix::objs::compute_hash(gix::hash::Kind::Sha1, gix::objs::Kind::Commit, body.as_bytes())
                .unwrap()
                .to_string();
            match seen.insert(id[..7].to_owned(), body.clone()) {
                Some(earlier) => Some((earlier, body)),
                None => None,
            }
        })
        .unwrap();
    let mut ids = Vec::new();
    for body in [one, two] {
        let mut child = Command::new("git")
            .args(["hash-object", "-w", "-t", "commit", "--stdin"])
            .current_dir(&repo)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .unwrap();
        use std::io::Write as _;
        child.stdin.take().unwrap().write_all(body.as_bytes()).unwrap();
        let output = child.wait_with_output().unwrap();
        ids.push(String::from_utf8(output.stdout).unwrap().trim().to_owned());
    }
    let ambiguous = service.commit(&repo, &ids[0][..7], &cancel).await.unwrap_err();
    assert_eq!(ambiguous.code(), "ambiguous");
    // A longer abbreviation names one; a commit on no branch is on none.
    let length = (8..=40).find(|length| ids[0][..*length] != ids[1][..*length]).unwrap();
    let one = service.commit(&repo, &ids[0][..length], &cancel).await.unwrap();
    assert_eq!((one.commit.id.as_str(), one.branches.len()), (ids[0].as_str(), 0));
}

// Under a second: a few git commands.
#[tokio::test]
async fn a_commit_is_on_a_branch_whose_later_commits_carry_earlier_dates() {
    let dir = tempfile::tempdir().unwrap();
    let repo = std::fs::canonicalize(dir.path()).unwrap();
    let service = GitService::default();
    let cancel = CancellationToken::new();
    git(&repo, &["init", "-q", "-b", "main"]);
    git_at(&repo, 1_700_003_000, &["commit", "-q", "--allow-empty", "-m", "start"]);
    git(&repo, &["checkout", "-q", "-b", "skewed"]);
    git_at(&repo, 1_700_002_000, &["commit", "-q", "--allow-empty", "-m", "dated later"]);
    let dated_later = git_output(&repo, &["rev-parse", "HEAD"]);
    // Its child carries an earlier date, as a skewed clock or a rebase leaves one.
    git_at(&repo, 1_700_001_000, &["commit", "-q", "--allow-empty", "-m", "dated earlier"]);
    git(&repo, &["checkout", "-q", "main"]);

    let commit = service.commit(&repo, &dated_later, &cancel).await.unwrap();
    assert_eq!(commit.branches, ["skewed"]);
}
