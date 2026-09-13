use std::{
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

fn summary(changes: &Changes) -> Vec<(String, ChangeKind, Option<String>, u64, u64)> {
    changes
        .files
        .iter()
        .map(|file| {
            (
                file.path.clone(),
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
            ("a.txt".into(), ChangeKind::Modified, None, 2, 1),
            ("b.txt".into(), ChangeKind::Deleted, None, 0, 2),
            ("d.txt".into(), ChangeKind::Added, None, 2, 0),
            (
                "dir/e.txt".into(),
                ChangeKind::Renamed,
                Some("dir/c.txt".into()),
                0,
                0
            ),
        ]
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
                ChangeKind::Renamed,
                Some("c.txt".into()),
                0,
                0
            ),
            ("new.txt".into(), ChangeKind::Added, None, 1, 0),
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

/// Polls until the watched baseline reflects `expected`, or fails after five seconds.
async fn wait_for(service: &GitService, root: &Path, expected: &[(&str, ChangeKind)]) -> Changes {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let result = changes(service, root).await;
        let actual: Vec<(String, ChangeKind)> = result
            .files
            .iter()
            .map(|file| (file.path.clone(), file.kind))
            .collect();
        let wanted: Vec<(String, ChangeKind)> = expected
            .iter()
            .map(|(path, kind)| ((*path).to_owned(), *kind))
            .collect();
        if actual == wanted {
            return result;
        }
        assert!(
            Instant::now() < deadline,
            "the watched baseline never reflected {expected:?}; last {actual:?}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
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
    assert_eq!(result["files"][0]["kind"], "added");
    assert_eq!(result["files"][0]["added"], 1);

    let reply = call(
        &service,
        Inbound::GitShow {
            id: "2".into(),
            root: ".".into(),
            path: "d.txt".into(),
        },
        &repo,
    )
    .await;
    assert_eq!(reply.kind, "git_error");
    assert_eq!(reply.code.as_deref(), Some("ENOENT"));
}
