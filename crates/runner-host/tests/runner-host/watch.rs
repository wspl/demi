//! Watching files and reading a file the backend holds (`runner.md`
//! § Watching files, § File contents), through the connection's Host server.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
    time::Duration,
};

use demi_runner_host::host::HostServer;
use demi_runner_process::pipes::PipeClient;
use demi_runner_protocol::wire::{self, FsResult, Inbound, Outbound, PipeRef};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};

/// A hang guard, not a latency check.
const GUARD: Duration = Duration::from_secs(60);

/// A Host server whose pipes go to `origin`, and the messages it sends.
fn server(root: &Path, origin: &str) -> (HostServer, mpsc::Receiver<wire::Frame>) {
    let (output, replies) = mpsc::channel(1024);
    // A pipe goes out with the device's token.
    let token = "device-token".parse().ok();
    let pipes = PipeClient::new(
        &origin.parse().unwrap(),
        tokio::sync::watch::Sender::new(token).subscribe(),
    )
    .unwrap();
    (HostServer::new(output, root.into(), pipes), replies)
}

async fn next(replies: &mut mpsc::Receiver<wire::Frame>) -> Outbound {
    wire::decode(&replies.recv().await.unwrap().into_bytes()).unwrap()
}

fn watch(host: &HostServer, path: &Path, recursive: bool) {
    host.handle_watch(Inbound::FsWatch {
        id: "w".into(),
        path: path.to_string_lossy().into_owned(),
        recursive,
    })
    .unwrap();
}

/// Waits for the watch's `ready`, which comes before anything it reports.
async fn ready(replies: &mut mpsc::Receiver<wire::Frame>) {
    match next(replies).await {
        Outbound::FsWatchReady { id } => assert_eq!(id, "w"),
        other => panic!("expected ready first, got {other:?}"),
    }
}

/// What the `changed` messages of a watch named.
#[derive(Default)]
struct Reported {
    paths: Vec<String>,
    /// Those of them that came, went or were renamed.
    entries: BTreeSet<String>,
    /// Those of them git ignores.
    ignored: BTreeSet<String>,
}

/// The paths of each `changed` until every one of `wanted` was reported,
/// each message checked to name a path once and to call ignored only paths
/// it names.
async fn until_reported(replies: &mut mpsc::Receiver<wire::Frame>, wanted: &[PathBuf]) -> Reported {
    let mut wanted: BTreeSet<String> = wanted.iter().map(|path| spelled(path)).collect();
    let mut reported = Reported::default();
    while !wanted.is_empty() {
        match next(replies).await {
            Outbound::FsWatchChanged { id, paths, entries, ignored } => {
                assert_eq!(id, "w");
                let unique: BTreeSet<&String> = paths.iter().collect();
                assert_eq!(unique.len(), paths.len(), "a path twice in {paths:?}");
                assert!(
                    ignored.iter().chain(&entries).all(|path| unique.contains(path)),
                    "{ignored:?} or {entries:?} not among {paths:?}"
                );
                reported.entries.extend(entries);
                for path in &paths {
                    wanted.remove(path);
                }
                reported.paths.extend(paths);
                reported.ignored.extend(ignored);
            }
            other => panic!("expected changed paths, got {other:?}"),
        }
    }
    reported
}

fn spelled(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

fn git(repo: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// Each `changed` says which of its paths git ignores (`web-api.md` § File
/// watch), so a page lists the working tree's changes again only for a
/// path that can be among them: a log a process appends to, or a build's
/// output, is ignored; a tracked file is not, whatever the rules say, nor is
/// anything in `.git`, which the changes follow too.
#[tokio::test(flavor = "multi_thread")]
async fn a_watch_names_the_changed_paths_git_ignores() {
    tokio::time::timeout(GUARD, async {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        git(&root, &["init", "-q", "-b", "main"]);
        std::fs::write(root.join(".gitignore"), "*.log\nbuild/\n").unwrap();
        let tracked_log = root.join("kept.log");
        std::fs::write(&tracked_log, "kept\n").unwrap();
        git(&root, &["add", "-f", "kept.log"]);
        std::fs::create_dir(root.join("build")).unwrap();
        let (host, mut replies) = server(&root, "http://127.0.0.1:1");
        watch(&host, &root, true);
        ready(&mut replies).await;

        let log = root.join("server.log");
        let output = root.join("build/app.js");
        let source = root.join("app.ts");
        let in_git = root.join(".git/probe.log");
        std::fs::write(&log, "line\n").unwrap();
        std::fs::write(&output, "built").unwrap();
        std::fs::write(&tracked_log, "kept\nmore\n").unwrap();
        std::fs::write(&source, "code").unwrap();
        std::fs::write(&in_git, "probe").unwrap();
        let reported = until_reported(
            &mut replies,
            &[log.clone(), output.clone(), tracked_log.clone(), source.clone(), in_git.clone()],
        )
        .await;
        assert!(reported.ignored.contains(&spelled(&log)), "{:?}", reported.ignored);
        assert!(reported.ignored.contains(&spelled(&output)), "{:?}", reported.ignored);
        for kept in [&tracked_log, &source, &in_git] {
            assert!(!reported.ignored.contains(&spelled(kept)), "{:?}", reported.ignored);
        }
        host.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_watch_reports_a_file_created_written_renamed_and_removed_under_the_path_it_was_given() {
    tokio::time::timeout(GUARD, async {
        let dir = tempfile::tempdir().unwrap();
        // The path as the backend names it, which on macOS is not the real
        // one the platform reports (`/var` is `/private/var`).
        let root = dir.path().to_path_buf();
        let (host, mut replies) = server(&root, "http://127.0.0.1:1");
        watch(&host, &root, true);
        ready(&mut replies).await;

        let first = root.join("a.txt");
        let second = root.join("b.txt");
        // A file that came, was renamed or went is an entry of its folder.
        std::fs::write(&first, "1\n").unwrap();
        let created = until_reported(&mut replies, std::slice::from_ref(&first)).await;
        assert!(created.entries.contains(&spelled(&first)), "{:?}", created.entries);
        std::fs::write(&first, "1\n2\n").unwrap();
        until_reported(&mut replies, std::slice::from_ref(&first)).await;
        std::fs::rename(&first, &second).unwrap();
        let renamed = until_reported(&mut replies, &[first.clone(), second.clone()]).await;
        assert!(renamed.entries.contains(&spelled(&second)), "{:?}", renamed.entries);
        std::fs::remove_file(&second).unwrap();
        let removed = until_reported(&mut replies, std::slice::from_ref(&second)).await;
        assert!(removed.entries.contains(&spelled(&second)), "{:?}", removed.entries);
        host.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_written_on_after_it_came_is_an_entry_only_when_it_came() {
    use std::io::Write as _;
    tokio::time::timeout(GUARD, async {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let (host, mut replies) = server(&root, "http://127.0.0.1:1");
        watch(&host, &root, true);
        ready(&mut replies).await;

        // A log a build makes and then appends to without a pause, each
        // line opened and closed as a shell's `>>` does, which FSEvents goes
        // on flagging as created.
        let log = root.join("build.log");
        let append = |line: &str| {
            let mut file = std::fs::OpenOptions::new().create(true).append(true).open(&log).unwrap();
            file.write_all(line.as_bytes()).unwrap();
        };
        append("1\n");
        let came = until_reported(&mut replies, std::slice::from_ref(&log)).await;
        assert!(came.entries.contains(&spelled(&log)), "{:?}", came.entries);
        for line in 2..5 {
            append(&format!("{line}\n"));
            let written = until_reported(&mut replies, std::slice::from_ref(&log)).await;
            assert!(!written.entries.contains(&spelled(&log)), "write {line}: {:?}", written.entries);
        }
        // Made anew under the same name, it came again.
        std::fs::remove_file(&log).unwrap();
        append("again\n");
        let again = until_reported(&mut replies, std::slice::from_ref(&log)).await;
        assert!(again.entries.contains(&spelled(&log)), "{:?}", again.entries);
        host.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_watch_of_one_folder_reports_its_entries_and_not_what_lies_below_them() {
    tokio::time::timeout(GUARD, async {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        std::fs::create_dir(root.join("sub")).unwrap();
        let (host, mut replies) = server(&root, "http://127.0.0.1:1");
        watch(&host, &root, false);
        ready(&mut replies).await;

        let deep = root.join("sub/deep.txt");
        let top = root.join("top.txt");
        // Reported in order: had the deeper file been, it would come first.
        std::fs::write(&deep, "deep").unwrap();
        std::fs::write(&top, "top").unwrap();
        let reported = until_reported(&mut replies, std::slice::from_ref(&top)).await.paths;
        assert!(
            !reported.contains(&deep.to_string_lossy().into_owned()),
            "{reported:?}"
        );
        host.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn writes_that_come_together_report_their_file_once() {
    tokio::time::timeout(GUARD, async {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let (host, mut replies) = server(&root, "http://127.0.0.1:1");
        watch(&host, &root, true);
        ready(&mut replies).await;

        let busy = root.join("busy.log");
        let marker = root.join("marker");
        const WRITES: usize = 20;
        for index in 0..WRITES {
            std::fs::write(&busy, format!("{index}\n")).unwrap();
        }
        std::fs::write(&marker, "").unwrap();
        let reported = until_reported(&mut replies, &[busy.clone(), marker]).await.paths;
        let busy = busy.to_string_lossy().into_owned();
        let times = reported.iter().filter(|path| **path == busy).count();
        assert!(times < WRITES, "reported {times} times");
        host.close().await;
    })
    .await
    .unwrap();
}

/// A pipe receiver that takes one upload and hands over its body.
async fn pipe_receiver() -> (String, tokio::sync::oneshot::Receiver<Vec<u8>>) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let (body, received) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = Vec::new();
        let mut buffer = [0; 4096];
        // The runner streams the body chunked, which ends with an empty
        // chunk.
        while !request.ends_with(b"0\r\n\r\n") {
            let read = socket.read(&mut buffer).await.unwrap();
            assert!(read > 0, "the upload ended early");
            request.extend_from_slice(&buffer[..read]);
        }
        // Handed over before the answer, which the runner's pipe end
        // reports after.
        let _ = body.send(request);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\n\r\n")
            .await
            .unwrap();
    });
    (origin, received)
}

fn read_file(host: &HostServer, path: &Path, version: Option<String>) {
    host.handle_filesystem(Inbound::FsReadFile {
        id: "r".into(),
        path: path.to_string_lossy().into_owned(),
        cwd: None,
        offset: None,
        length: None,
        version,
        output: PipeRef {
            id: "p".into(),
            url: "/api/pipes/p".into(),
        },
    })
    .unwrap();
}

/// The opened file the read answers, then how its pipe ended.
async fn read_answer(replies: &mut mpsc::Receiver<wire::Frame>) -> (wire::OpenedFile, bool) {
    let opened = match next(replies).await {
        Outbound::FsOk(wire::FsOk {
            result: FsResult::ReadFile(opened),
            ..
        }) => opened,
        other => panic!("expected the opened file, got {other:?}"),
    };
    match next(replies).await {
        Outbound::PipeDone { pipe_id, ok, .. } => {
            assert_eq!(pipe_id, "p");
            (opened, ok)
        }
        other => panic!("expected the pipe's end, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_that_holds_the_files_version_streams_nothing_and_one_that_holds_an_old_one_streams() {
    tokio::time::timeout(GUARD, async {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("note.txt");
        std::fs::write(&file, "hello").unwrap();
        let (origin, mut uploaded) = pipe_receiver().await;
        let (host, mut replies) = server(dir.path(), &origin);

        read_file(&host, &file, None);
        let (first, streamed) = read_answer(&mut replies).await;
        assert!(!first.unchanged);
        assert!(streamed);
        let body = uploaded.try_recv().unwrap();
        assert!(body.windows(5).any(|bytes| bytes == b"hello"));

        // The receiver took its one upload: a second would find no one.
        read_file(&host, &file, Some(first.version.clone()));
        let (held, streamed) = read_answer(&mut replies).await;
        assert!(held.unchanged);
        assert_eq!(held.version, first.version);
        assert!(!streamed);

        let (origin, uploaded) = pipe_receiver().await;
        let (host, mut replies) = server(dir.path(), &origin);
        read_file(&host, &file, Some("W/\"0-0\"".into()));
        let (old, streamed) = read_answer(&mut replies).await;
        assert!(!old.unchanged);
        assert_eq!(old.version, first.version);
        assert!(streamed);
        let body = uploaded.await.unwrap();
        assert!(body.windows(5).any(|bytes| bytes == b"hello"));
    })
    .await
    .unwrap();
}
