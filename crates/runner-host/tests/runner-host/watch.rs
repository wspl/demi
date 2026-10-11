//! Subscriptions and reading a file the backend holds (`runner.md`
//! § Watching files, § File contents), through the connection's Host
//! server: what each kind of subscription changes for, the numbers that
//! order reads and changes, lost events, leases, and what git ignores.

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::Duration,
};

use demi_runner_host::host::HostServer;
use demi_runner_process::pipes::PipeClient;
use demi_runner_protocol::files::SubscriptionKind;
use demi_runner_protocol::wire::{self, FsResult, Inbound, Outbound, PipeRef, ReadAt};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;

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

fn subscribe(host: &HostServer, id: &str, kind: SubscriptionKind, path: &Path) {
    host.handle_watch(Inbound::FsSubscribe {
        id: id.into(),
        kind,
        path: path.to_string_lossy().into_owned(),
    })
    .unwrap();
}

/// Waits for each of `ids` to be live, which comes before anything it
/// reports: the number each started at.
async fn live(replies: &mut mpsc::Receiver<wire::Frame>, ids: &[&str]) -> BTreeMap<String, u64> {
    let mut waiting: BTreeSet<&str> = ids.iter().copied().collect();
    let mut since = BTreeMap::new();
    while !waiting.is_empty() {
        match next(replies).await {
            Outbound::FsLive { id, since: at, epoch } => {
                assert!(waiting.remove(id.as_str()), "{id} live twice or unasked");
                assert_eq!(epoch, demi_runner_host::numbers::epoch());
                since.insert(id, at);
            }
            other => panic!("expected live first, got {other:?}"),
        }
    }
    since
}

/// The batches the runner reports until each of `wanted` changed, each
/// checked to name an id once: every id they named, with the latest number.
async fn changed(replies: &mut mpsc::Receiver<wire::Frame>, wanted: &[&str]) -> BTreeMap<String, u64> {
    let mut waiting: BTreeSet<&str> = wanted.iter().copied().collect();
    let mut seen = BTreeMap::new();
    while !waiting.is_empty() {
        match next(replies).await {
            Outbound::FsChanged { changes } => {
                let ids: BTreeSet<&String> = changes.iter().map(|(id, _)| id).collect();
                assert_eq!(ids.len(), changes.len(), "an id twice in {changes:?}");
                for (id, at) in changes {
                    waiting.remove(id.as_str());
                    let latest = seen.entry(id).or_insert(at);
                    *latest = (*latest).max(at);
                }
            }
            other => panic!("expected changes, got {other:?}"),
        }
    }
    seen
}

/// The number a read of `path` answers with: the Host's before it read.
async fn read_at(host: &HostServer, replies: &mut mpsc::Receiver<wire::Frame>, path: &Path) -> ReadAt {
    host.handle_filesystem(Inbound::FsStat {
        id: "stat".into(),
        path: path.to_string_lossy().into_owned(),
        cwd: None,
    })
    .unwrap();
    loop {
        match next(replies).await {
            Outbound::FsOk(wire::FsOk { id, read_at, .. }) if id == "stat" => return read_at,
            Outbound::FsError { id, read_at, .. } if id == "stat" => return read_at,
            // A change of a subscription may come in between.
            Outbound::FsChanged { .. } => {}
            other => panic!("expected the read's answer, got {other:?}"),
        }
    }
}

fn git(repo: &Path, args: &[&str]) {
    let status = std::process::Command::new("git")
        .args(args)
        .current_dir(repo)
        .stdout(std::process::Stdio::null())
        .status()
        .unwrap();
    assert!(status.success(), "git {args:?}");
}

/// A repository with one commit of `tracked.txt`, which ignores `target/`.
fn repository() -> (tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    git(&root, &["init", "-q", "-b", "main"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "Test"]);
    std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
    std::fs::write(root.join("tracked.txt"), "1\n").unwrap();
    git(&root, &["add", "."]);
    git(&root, &["commit", "-q", "-m", "first"]);
    (dir, root)
}

/// A file's subscription changes when its file is written, replaced by a
/// rename as an editor saves, removed and made again, and not when a file
/// beside it changes. Outside a working tree it is watched alone, on macOS
/// with a kqueue watched again under its name each time a rename replaces
/// the file.
#[tokio::test(flavor = "multi_thread")]
async fn a_file_subscription_follows_its_file_through_writes_renames_over_it_and_removal() {
    tokio::time::timeout(GUARD, async {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let notes = root.join("notes.txt");
        let beside = root.join("other.txt");
        std::fs::write(&notes, "1\n").unwrap();
        std::fs::write(&beside, "x\n").unwrap();
        let (host, mut replies) = server(&root, "http://127.0.0.1:1");
        subscribe(&host, "notes", SubscriptionKind::File, &notes);
        subscribe(&host, "other", SubscriptionKind::File, &beside);
        live(&mut replies, &["notes", "other"]).await;

        std::fs::write(&notes, "2\n").unwrap();
        let written = changed(&mut replies, &["notes"]).await;
        assert!(!written.contains_key("other"), "{written:?}");
        for round in 0..2 {
            let saved = root.join(format!(".notes-{round}"));
            std::fs::write(&saved, format!("saved {round}\n")).unwrap();
            std::fs::rename(&saved, &notes).unwrap();
            let replaced = changed(&mut replies, &["notes"]).await;
            assert!(!replaced.contains_key("other"), "round {round}: {replaced:?}");
        }
        std::fs::remove_file(&notes).unwrap();
        changed(&mut replies, &["notes"]).await;
        std::fs::write(&notes, "again\n").unwrap();
        changed(&mut replies, &["notes"]).await;
        std::fs::write(&beside, "y\n").unwrap();
        let other = changed(&mut replies, &["other"]).await;
        assert!(!other.contains_key("notes"), "{other:?}");
        host.close().await;
    })
    .await
    .unwrap();
}

/// A folder's names change when an entry comes, goes or is renamed, and not
/// when one is written in place; its details, which show sizes and times,
/// change for that write too.
#[tokio::test(flavor = "multi_thread")]
async fn a_folder_hears_its_names_alone_or_its_entries_contents_too() {
    tokio::time::timeout(GUARD, async {
        let dir = tempfile::tempdir().unwrap();
        let listed = dir.path().join("listed");
        std::fs::create_dir(&listed).unwrap();
        std::fs::write(listed.join("a.log"), "1\n").unwrap();
        let (host, mut replies) = server(dir.path(), "http://127.0.0.1:1");
        subscribe(&host, "names", SubscriptionKind::FolderNames, &listed);
        subscribe(&host, "details", SubscriptionKind::FolderDetails, &listed);
        live(&mut replies, &["names", "details"]).await;

        use std::io::Write as _;
        let mut log = std::fs::OpenOptions::new().append(true).open(listed.join("a.log")).unwrap();
        log.write_all(b"2\n").unwrap();
        drop(log);
        let written = changed(&mut replies, &["details"]).await;
        assert!(!written.contains_key("names"), "{written:?}");
        std::fs::write(listed.join("b.txt"), "b").unwrap();
        changed(&mut replies, &["names", "details"]).await;
        std::fs::rename(listed.join("b.txt"), listed.join("c.txt")).unwrap();
        changed(&mut replies, &["names", "details"]).await;
        std::fs::remove_file(listed.join("c.txt")).unwrap();
        changed(&mut replies, &["names", "details"]).await;
        host.close().await;
    })
    .await
    .unwrap();
}

/// A working tree's subscription changes for what git would list again: an
/// edit, a mode changed, a staging; its last commit's subscription changes
/// with a commit, not with an edit.
#[tokio::test(flavor = "multi_thread")]
async fn the_working_tree_changes_for_edits_modes_and_staging_and_its_last_commit_with_a_commit() {
    tokio::time::timeout(GUARD, async {
        let (_dir, root) = repository();
        let (host, mut replies) = server(&root, "http://127.0.0.1:1");
        subscribe(&host, "tree", SubscriptionKind::Worktree, &root);
        subscribe(&host, "head", SubscriptionKind::Head, &root);
        live(&mut replies, &["tree", "head"]).await;

        let tracked = root.join("tracked.txt");
        std::fs::write(&tracked, "1\n2\n").unwrap();
        let edited = changed(&mut replies, &["tree"]).await;
        assert!(!edited.contains_key("head"), "{edited:?}");
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&tracked, std::fs::Permissions::from_mode(0o755)).unwrap();
        let moded = changed(&mut replies, &["tree"]).await;
        assert!(!moded.contains_key("head"), "{moded:?}");
        git(&root, &["add", "tracked.txt"]);
        changed(&mut replies, &["tree"]).await;
        git(&root, &["commit", "-q", "-m", "second"]);
        changed(&mut replies, &["head"]).await;
        host.close().await;
    })
    .await
    .unwrap();
}

/// A build that writes 20,000 files under the ignored `target/` changes no
/// working tree subscription and costs the working tree's changes no walk
/// of the whole tree, while a subscription of a log under `target/`, as a
/// page shows one, changes as any other (`runner.md` § Watching files).
/// `target/` comes after the tree's watch started, so the watch hears the
/// build and judges each path by git's rules. Costs 20,000 small writes,
/// about a second.
#[tokio::test(flavor = "multi_thread")]
async fn a_build_under_an_ignored_folder_changes_nothing_but_what_is_subscribed_inside_it() {
    tokio::time::timeout(GUARD, async {
        let (_dir, root) = repository();
        let (host, mut replies) = server(&root, "http://127.0.0.1:1");
        let log = root.join("target/build.log");
        subscribe(&host, "tree", SubscriptionKind::Worktree, &root);
        subscribe(&host, "log", SubscriptionKind::File, &log);
        live(&mut replies, &["tree", "log"]).await;
        // The working tree's changes are known and watched: the requests
        // that follow are answered from what the watch records.
        let cancel = CancellationToken::new();
        while !host.git().changes(&root, &cancel).await.unwrap().watched {}
        host.git().changes(&root, &cancel).await.unwrap();
        let walks = host.git().whole_walks();

        let output = root.join("target/debug/deps");
        std::fs::create_dir_all(&output).unwrap();
        tokio::task::spawn_blocking(move || {
            for index in 0..20_000 {
                std::fs::write(output.join(format!("unit-{index}.o")), "o").unwrap();
            }
        })
        .await
        .unwrap();
        std::fs::write(&log, "built\n").unwrap();
        // The log's change comes after the build's on the same watch.
        let heard = changed(&mut replies, &["log"]).await;
        assert!(!heard.contains_key("tree"), "{heard:?}");
        let changes = host.git().changes(&root, &cancel).await.unwrap();
        assert!(changes.files.is_empty(), "{:?}", changes.files);
        assert_eq!(host.git().whole_walks(), walks, "the build made a walk of the whole tree");

        std::fs::write(root.join("tracked.txt"), "edited\n").unwrap();
        changed(&mut replies, &["tree"]).await;
        host.close().await;
    })
    .await
    .unwrap();
}

/// A folder git ignores that is there when the tree's watch starts is left
/// out of it, and a subscription inside it gets a watch of its own, which
/// hears the folder's writes the tree's watch no longer receives.
#[tokio::test(flavor = "multi_thread")]
async fn a_subscription_inside_a_folder_the_tree_leaves_out_has_a_watch_of_its_own() {
    tokio::time::timeout(GUARD, async {
        let (_dir, root) = repository();
        let log = root.join("target/build.log");
        std::fs::create_dir(root.join("target")).unwrap();
        std::fs::write(&log, "0\n").unwrap();
        let (host, mut replies) = server(&root, "http://127.0.0.1:1");
        subscribe(&host, "tree", SubscriptionKind::Worktree, &root);
        subscribe(&host, "log", SubscriptionKind::File, &log);
        live(&mut replies, &["tree", "log"]).await;

        std::fs::write(&log, "1\n").unwrap();
        let heard = changed(&mut replies, &["log"]).await;
        assert!(!heard.contains_key("tree"), "{heard:?}");
        std::fs::write(root.join("tracked.txt"), "edited\n").unwrap();
        let edited = changed(&mut replies, &["tree"]).await;
        assert!(!edited.contains_key("log"), "{edited:?}");
        host.close().await;
    })
    .await
    .unwrap();
}

/// Every read answers the Host's number taken before it read, and every
/// change the number of the event that made it, one count for the whole
/// process: a change after a read always has a higher number than the
/// read, and a read after the change has one no lower, so the page can tell
/// from the numbers alone whether what it holds is current, inside a
/// working tree's watch and outside it.
#[tokio::test(flavor = "multi_thread")]
async fn a_change_after_a_read_is_numbered_above_it_and_a_read_after_it_is_not_below_it() {
    tokio::time::timeout(GUARD, async {
        let (_dir, root) = repository();
        let outside = tempfile::tempdir().unwrap();
        let inner = root.join("tracked.txt");
        let alone = outside.path().join("alone.txt");
        std::fs::write(&alone, "1\n").unwrap();
        let (host, mut replies) = server(&root, "http://127.0.0.1:1");
        subscribe(&host, "tree", SubscriptionKind::Worktree, &root);
        subscribe(&host, "inner", SubscriptionKind::File, &inner);
        subscribe(&host, "alone", SubscriptionKind::File, &alone);
        let since = live(&mut replies, &["tree", "inner", "alone"]).await;

        for (id, file) in [("inner", &inner), ("alone", &alone)] {
            let before = read_at(&host, &mut replies, file).await;
            assert_eq!(before.epoch, demi_runner_host::numbers::epoch());
            assert!(before.number >= since[id], "{id}: read at {before:?}, live since {}", since[id]);
            std::fs::write(file, format!("{id} changed\n")).unwrap();
            let at = changed(&mut replies, &[id]).await[id];
            assert!(at > before.number, "{id}: changed at {at}, read at {}", before.number);
            let after = read_at(&host, &mut replies, file).await;
            assert!(after.number >= at, "{id}: read at {} after a change at {at}", after.number);
        }
        host.close().await;
    })
    .await
    .unwrap();
}

/// Events the platform says it dropped change the subscriptions they
/// concern and no others: a folder FSEvents asks to rescan changes what
/// lies under it and the working tree, unless git ignores the folder; a
/// drop of a stream's own changes every subscription on that stream and
/// none on another, so a drop outside the working tree reads nothing inside
/// it. Nothing can make FSEvents drop events, so the test speaks for it
/// through the way its events take.
#[cfg(target_os = "macos")]
#[tokio::test(flavor = "multi_thread")]
async fn lost_events_change_only_the_subscriptions_they_concern() {
    use fsevent_flags::*;
    tokio::time::timeout(GUARD, async {
        let (_dir, root) = repository();
        let outside = tempfile::tempdir().unwrap();
        for folder in ["src", "docs", "target"] {
            std::fs::create_dir(root.join(folder)).unwrap();
        }
        std::fs::write(root.join("src/a.rs"), "a").unwrap();
        std::fs::write(root.join("docs/b.md"), "b").unwrap();
        let (host, mut replies) = server(&root, "http://127.0.0.1:1");
        subscribe(&host, "tree", SubscriptionKind::Worktree, &root);
        subscribe(&host, "a", SubscriptionKind::File, &root.join("src/a.rs"));
        subscribe(&host, "b", SubscriptionKind::File, &root.join("docs/b.md"));
        subscribe(&host, "outside", SubscriptionKind::FolderDetails, outside.path());
        live(&mut replies, &["tree", "a", "b", "outside"]).await;
        // FSEvents may still bring what the setup wrote before the streams
        // started; once a write after it is heard, all of that has come.
        std::fs::write(root.join("docs/sentinel.md"), "s").unwrap();
        std::fs::write(outside.path().join("sentinel.txt"), "s").unwrap();
        changed(&mut replies, &["tree", "outside"]).await;
        let real = std::fs::canonicalize(&root).unwrap();
        let ids = |message: Outbound| match message {
            Outbound::FsChanged { changes } => changes.into_iter().map(|(id, _)| id).collect::<BTreeSet<_>>(),
            other => panic!("expected changes, got {other:?}"),
        };
        let set = |ids: &[&str]| ids.iter().map(|id| id.to_string()).collect::<BTreeSet<_>>();

        assert!(host.watches().said(&root, &real.join("src"), MUST_SCAN_SUB_DIRS));
        assert_eq!(ids(next(&mut replies).await), set(&["a", "tree"]));
        // A folder git ignores: nothing subscribed lies under it, and the
        // working tree does not list it. The outside folder's own change,
        // which comes after on its own stream, shows nothing came first.
        assert!(host.watches().said(&root, &real.join("target"), MUST_SCAN_SUB_DIRS));
        std::fs::write(outside.path().join("new.txt"), "n").unwrap();
        let after = changed(&mut replies, &["outside"]).await;
        assert!(!after.contains_key("tree"), "{after:?}");
        assert!(host.watches().said(&root, &real, USER_DROPPED));
        assert_eq!(ids(next(&mut replies).await), set(&["a", "b", "tree"]));
        assert!(host.watches().said(outside.path(), &std::fs::canonicalize(outside.path()).unwrap(), KERNEL_DROPPED));
        assert_eq!(ids(next(&mut replies).await), set(&["outside"]));
        host.close().await;
    })
    .await
    .unwrap();
}

/// FSEvents' flags for a dropped event, as `fsevent-sys` names them.
#[cfg(target_os = "macos")]
mod fsevent_flags {
    pub const MUST_SCAN_SUB_DIRS: u32 = 0x0000_0001;
    pub const USER_DROPPED: u32 = 0x0000_0002;
    pub const KERNEL_DROPPED: u32 = 0x0000_0004;
}

/// A subscription is a lease that runs 10 minutes from its subscribe or its
/// last renew: one not renewed ends when its lease runs out, and an
/// unsubscribe ends one at once (`runner.md` § Watching files). The
/// runtime's clock is held still while the leases run.
#[tokio::test]
async fn a_subscription_not_renewed_ends_with_its_lease_and_one_unsubscribed_at_once() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("watched.txt");
    std::fs::write(&file, "1\n").unwrap();
    let (host, mut replies) = server(dir.path(), "http://127.0.0.1:1");
    for id in ["lapsed", "renewed", "ended"] {
        subscribe(&host, id, SubscriptionKind::File, &file);
    }
    live(&mut replies, &["lapsed", "renewed", "ended"]).await;

    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(6 * 60)).await;
    host.handle_watch(Inbound::FsRenew {
        ids: vec!["renewed".into(), "ended".into()],
    })
    .unwrap();
    // A subscription after the renew is live once the renew was taken.
    subscribe(&host, "later", SubscriptionKind::File, &file);
    live(&mut replies, &["later"]).await;
    tokio::time::advance(Duration::from_secs(5 * 60)).await;
    host.handle_watch(Inbound::FsUnsubscribe { id: "ended".into() }).unwrap();
    tokio::time::resume();

    std::fs::write(&file, "2\n").unwrap();
    let heard = tokio::time::timeout(GUARD, changed(&mut replies, &["renewed"])).await.unwrap();
    let heard: BTreeSet<&str> = heard.keys().map(String::as_str).collect();
    assert_eq!(heard, BTreeSet::from(["later", "renewed"]));
    host.close().await;
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
