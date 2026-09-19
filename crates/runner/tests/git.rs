use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

use demi_runner::connection::wire::Inbound;
use demi_runner::git::{ChangeKind, Changes, GitError, GitService};
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
        .show(dir.path(), "a.txt", &CancellationToken::new())
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
        .show(&repo.join("dir"), "c.txt", &CancellationToken::new())
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
        service.show(&repo, "a.txt", &cancel).await.unwrap(),
        b"1\n2\n3\n"
    );
    let missing = service
        .show(&repo, "untracked.txt", &cancel)
        .await
        .unwrap_err();
    assert_eq!(missing.code(), "ENOENT");
    let directory = service.show(&repo, "dir", &cancel).await.unwrap_err();
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
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let result = changes(service, root).await;
        if settled(&result) {
            return result;
        }
        assert!(
            Instant::now() < deadline,
            "the watched baseline never settled; last {:?}",
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
    let first = changes(&service, &repo).await;
    assert!(first.watched, "the watch starts with the first request");
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
async fn a_staged_rename_stays_one_entry_across_watched_requests() {
    let (_dir, repo) = committed_repo();
    git(&repo, &["mv", "a.txt", "moved.txt"]);
    let service = GitService::default();
    let first = changes(&service, &repo).await;
    assert!(first.watched);
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

#[derive(Deserialize)]
struct Reply {
    #[serde(rename = "type")]
    kind: String,
    result: Option<serde_json::Value>,
    code: Option<String>,
}

async fn call(service: &GitService, message: Inbound, cwd: &Path) -> Reply {
    let bytes = demi_runner::git::handle(service, &message, cwd, &CancellationToken::new())
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
