//! A conversation's files through its host access (`web-api.md` § Device
//! files and remote references, § File text and working tree changes;
//! `sessions-and-targets.md` § Host operations): listings, directories,
//! deletes and text; a file's bytes by range under the headers that keep
//! them inert, and back as an upload; the working tree's changes; what the
//! host access refuses; and the transfer a shutdown ends. The conversation
//! works in a directory of a paired device's real runner, which a workspace
//! written to the control database names, as the workspace routes would.

use std::path::{Path, PathBuf};
use std::time::Duration;

use bytes::Bytes;
use demi_runner_protocol::wire::MAX_MESSAGE_BYTES;
use demi_web_api::auth::Role;
use demi_web_api::error::ErrorCode;
use futures_util::{StreamExt as _, stream};
use reqwest::{Method, StatusCode};
use serde_json::{Value, json};

use crate::support::{Answer, Harness, Paired, Session, TestBackend, answer, eventually};

const CONVERSATION: &str = "5a1d0c3e-8f3a-4c1e-9d2b-7a1c2e3f4a01";

/// A conversation whose work runs in `root`, a directory in the home of a
/// paired device's runner.
struct OnDevice {
    harness: Harness,
    backend: TestBackend,
    master: Session,
    paired: Paired,
    root: PathBuf,
}

impl OnDevice {
    async fn start() -> Self {
        let harness = Harness::new();
        let (backend, master) = harness.start_set_up().await;
        let paired = backend.pair(&master, "laptop").await;
        let created = backend
            .post("/api/conversations", Some(&master), json!({ "id": CONVERSATION }))
            .await;
        assert_eq!(created.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&created.body));
        let root = paired.runner.home_dir().join("work");
        std::fs::create_dir_all(&root).unwrap();
        let control = harness.control_database();
        control
            .execute(
                "INSERT INTO workspaces (id, user_id, device_id, path, name, sort_order, created_at)
                 VALUES ('workspace-1', ?1, ?2, ?3, 'work', 0, 0)",
                rusqlite::params![master.user.id.as_str(), paired.id(), root.to_str().unwrap()],
            )
            .unwrap();
        control
            .execute(
                "UPDATE conversations SET target_kind = 'workspace', target_workspace_id = 'workspace-1'
                 WHERE id = ?1",
                [CONVERSATION],
            )
            .unwrap();
        Self {
            harness,
            backend,
            master,
            paired,
            root,
        }
    }

    /// `relative` in the conversation's directory.
    fn path(&self, relative: &str) -> String {
        self.root.join(relative).to_str().unwrap().to_owned()
    }

    /// A request of the master's to one of the conversation's routes.
    async fn call(&self, method: Method, route: &str, headers: &[(&str, &str)], body: Option<reqwest::Body>) -> Answer {
        let path = format!("/api/conversations/{CONVERSATION}{route}");
        answer(self.backend.response(method, &path, &self.master, headers, body).await).await
    }

    async fn get(&self, route: &str) -> Answer {
        self.call(Method::GET, route, &[], None).await
    }

    fn execute(&self, sql: &str, parameters: &[&str]) {
        self.harness
            .control_database()
            .execute(sql, rusqlite::params_from_iter(parameters))
            .unwrap();
    }
}

/// `pairs` as a query string.
fn query(pairs: &[(&str, &str)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(pairs)
        .finish()
}

fn code(answer: &Answer) -> ErrorCode {
    answer.error().code
}

fn git(directory: &Path, arguments: &[&str]) {
    let status = std::process::Command::new("git")
        .args(["-c", "commit.gpgsign=false", "-c", "core.hooksPath=/dev/null"])
        .args(arguments)
        .current_dir(directory)
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.com")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.com")
        .status()
        .unwrap();
    assert!(status.success(), "git {arguments:?}");
}

/// Bytes that differ at every position, so a misplaced range shows.
fn pattern(size: usize) -> Vec<u8> {
    (0..size).map(|index| ((index * 31 + (index >> 8)) % 251) as u8).collect()
}

fn header<'a>(answer: &'a Answer, name: &str) -> Option<&'a str> {
    answer.headers.get(name).map(|value| value.to_str().unwrap())
}

#[tokio::test]
async fn the_working_tree_lists_its_changes_and_reads_one_file_and_an_offline_device_says_so() {
    let mut device = OnDevice::start().await;
    let root = device.root.to_str().unwrap().to_owned();

    // Not a repository yet: an answer, not an error.
    let outside = device.get("/changes").await;
    assert_eq!(outside.status, StatusCode::OK, "{}", String::from_utf8_lossy(&outside.body));
    let outside: Value = outside.json();
    assert_eq!(
        (&outside["root"], &outside["repository"], &outside["files"]),
        (&json!(root), &json!(false), &json!([]))
    );

    git(&device.root, &["init", "-q", "-b", "main"]);
    std::fs::write(device.root.join("a.txt"), "1\n2\n").unwrap();
    git(&device.root, &["add", "."]);
    git(&device.root, &["commit", "-q", "-m", "first"]);
    std::fs::write(device.root.join("a.txt"), "1\n2\n3\n").unwrap();
    std::fs::write(device.root.join("b.txt"), "new\n").unwrap();
    std::fs::write(device.root.join("blob.bin"), [0, 255, 1]).unwrap();
    let changes: Value = device.get("/changes").await.json();
    assert_eq!((&changes["repository"], &changes["truncated"]), (&json!(true), &json!(false)));
    let head = changes["head"].as_str().unwrap();
    assert!(head.len() == 40 && head.chars().all(|char| char.is_ascii_hexdigit()), "{head}");
    assert_eq!(
        changes["files"],
        json!([
            { "path": "a.txt", "status": " M", "kind": "modified", "added": 1, "removed": 0 },
            { "path": "b.txt", "status": "??", "kind": "added", "added": 1, "removed": 0 },
            { "path": "blob.bin", "status": "??", "kind": "added", "added": 0, "removed": 0 },
        ])
    );

    let modified = device.get("/changes/file?path=a.txt").await;
    assert_eq!(modified.json::<Value>(), json!({ "original": "1\n2\n", "modified": "1\n2\n3\n" }));
    let added = device.get("/changes/file?path=b.txt").await;
    assert_eq!(added.json::<Value>(), json!({ "original": "", "modified": "new\n" }));
    let binary = device.get("/changes/file?path=blob.bin").await;
    assert_eq!((binary.status, code(&binary)), (StatusCode::UNSUPPORTED_MEDIA_TYPE, ErrorCode::NotText));
    let escaping = device.get("/changes/file?path=../x").await;
    assert_eq!(escaping.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery));

    let a = device.path("a.txt");
    let text = device.get(&format!("/fs/file?{}", query(&[("path", &a)]))).await;
    assert_eq!(text.json::<Value>(), json!({ "path": a, "text": "1\n2\n3\n" }));
    let missing = device.get(&format!("/fs/file?{}", query(&[("path", &device.path("nope"))]))).await;
    assert_eq!(missing.refusal(), (StatusCode::NOT_FOUND, ErrorCode::FsError));

    // The listing starts in the conversation's directory and names the
    // device's home; a directory is made with its parents.
    let made = device.call(Method::POST, "/fs", &[("content-type", "application/json")], Some(
        json!({ "path": device.path("made/deep") }).to_string().into(),
    ))
    .await;
    assert_eq!(made.status, StatusCode::CREATED, "{}", String::from_utf8_lossy(&made.body));
    assert!(device.root.join("made/deep").is_dir());
    let listing: Value = device.get("/fs").await.json();
    assert_eq!((&listing["path"], &listing["home"]), (&json!(root), &json!(device.paired.runner.home())));
    let mut names: Vec<&str> = listing["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["name"].as_str().unwrap())
        .collect();
    names.sort_unstable();
    assert_eq!(names, [".git", "a.txt", "b.txt", "blob.bin", "made"]);

    // A listing over the runner's message limit fails that request alone.
    let crowded = device.root.join("crowded");
    std::fs::create_dir(&crowded).unwrap();
    let name = "n".repeat(200);
    for index in 0..=MAX_MESSAGE_BYTES / 200 {
        std::fs::write(crowded.join(format!("{name}{index}")), "").unwrap();
    }
    let listing = device.get(&format!("/fs?{}", query(&[("path", crowded.to_str().unwrap())]))).await;
    assert_eq!(listing.refusal(), (StatusCode::PAYLOAD_TOO_LARGE, ErrorCode::DirectoryTooLarge));
    assert_eq!(device.get(&format!("/fs/file?{}", query(&[("path", &a)]))).await.status, StatusCode::OK);

    // Offline: the routes say so rather than waking anything.
    device.paired.runner.stop().await;
    device.backend.until_online(&device.master, device.paired.id(), false).await;
    let refused = device.get("/changes").await;
    assert_eq!(refused.refusal(), (StatusCode::CONFLICT, ErrorCode::DeviceOffline));
    device.backend.close().await;
}

#[tokio::test]
async fn the_raw_routes_stream_a_file_by_range_under_inert_headers_and_the_committed_side_from_git() {
    let device = OnDevice::start().await;
    let raw = |path: &str, extra: &[(&str, &str)]| {
        let mut pairs = vec![("path", device.path(path))];
        pairs.extend(extra.iter().map(|(name, value)| (*name, (*value).to_owned())));
        let pairs: Vec<(&str, &str)> = pairs.iter().map(|(name, value)| (*name, value.as_str())).collect();
        format!("/fs/raw?{}", query(&pairs))
    };
    let image = pattern(300_000);
    std::fs::write(device.root.join("logo.svg"), &image).unwrap();

    let whole = device.get(&raw("logo.svg", &[])).await;
    assert_eq!(whole.status, StatusCode::OK);
    assert_eq!(header(&whole, "content-type"), Some("image/svg+xml"));
    assert_eq!(
        header(&whole, "content-security-policy"),
        Some("default-src 'none'; style-src 'unsafe-inline'; sandbox")
    );
    assert_eq!(header(&whole, "x-content-type-options"), Some("nosniff"));
    assert_eq!(header(&whole, "x-accel-buffering"), Some("no"));
    assert_eq!(header(&whole, "cache-control"), Some("private, no-cache"));
    assert_eq!(header(&whole, "accept-ranges"), Some("bytes"));
    let etag = header(&whole, "etag").unwrap().to_owned();
    let (size, time) = etag
        .strip_prefix("W/\"")
        .and_then(|tag| tag.strip_suffix('"'))
        .and_then(|tag| tag.split_once('-'))
        .unwrap();
    assert!([size, time].iter().all(|part| part.chars().all(|char| char.is_ascii_hexdigit())), "{etag}");
    assert_eq!(whole.body, image);

    let head = device.call(Method::HEAD, &raw("logo.svg", &[]), &[], None).await;
    assert_eq!(head.status, StatusCode::OK);
    assert_eq!(header(&head, "content-length"), Some("300000"));
    assert_eq!(header(&head, "etag"), Some(etag.as_str()));
    assert!(header(&head, "last-modified").is_some());

    let part = device.call(Method::GET, &raw("logo.svg", &[]), &[("range", "bytes=1000-1999")], None).await;
    assert_eq!(part.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(header(&part, "content-range"), Some("bytes 1000-1999/300000"));
    assert_eq!(part.body, image[1000..2000]);
    let past = device.call(Method::GET, &raw("logo.svg", &[]), &[("range", "bytes=300000-")], None).await;
    assert_eq!(past.status, StatusCode::RANGE_NOT_SATISFIABLE);

    let unchanged = device.call(Method::GET, &raw("logo.svg", &[]), &[("if-none-match", &etag)], None).await;
    assert_eq!(unchanged.status, StatusCode::NOT_MODIFIED);
    assert_eq!(device.get(&raw("logo.svg", &[("version", &etag)])).await.status, StatusCode::OK);
    std::fs::write(device.root.join("logo.svg"), pattern(10)).unwrap();
    let changed = device.get(&raw("logo.svg", &[("version", &etag)])).await;
    assert_eq!(changed.refusal(), (StatusCode::PRECONDITION_FAILED, ErrorCode::FileChanged));

    let download = device.get(&raw("logo.svg", &[("download", "true")])).await;
    assert_eq!(header(&download, "content-type"), Some("application/octet-stream"));
    assert!(header(&download, "content-disposition").unwrap().starts_with("attachment; filename=\"logo.svg\""));
    assert_eq!(header(&download, "content-security-policy"), None);
    std::fs::write(device.root.join("page.html"), "<script>alert(1)</script>").unwrap();
    let html = device.get(&raw("page.html", &[])).await;
    assert_eq!(header(&html, "content-type"), Some("application/octet-stream"));
    assert!(header(&html, "content-disposition").unwrap().starts_with("attachment"));
    assert_eq!(device.get(&raw(".", &[])).await.refusal(), (StatusCode::NOT_FOUND, ErrorCode::NotFound));
    assert_eq!(device.get(&raw("missing.png", &[])).await.refusal(), (StatusCode::NOT_FOUND, ErrorCode::FsError));
    let flag = device.get(&raw("logo.svg", &[("download", "1")])).await;
    assert_eq!(flag.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery));

    // Far past the runner's message limit, whole and in order; streamed,
    // the answer has no length, which a HEAD reports.
    let video = pattern(3 * MAX_MESSAGE_BYTES + 5);
    std::fs::write(device.root.join("demo.mp4"), &video).unwrap();
    let streamed = device.get(&raw("demo.mp4", &[])).await;
    assert_eq!(header(&streamed, "content-type"), Some("video/mp4"));
    assert_eq!(header(&streamed, "content-length"), None);
    assert_eq!(header(&streamed, "content-security-policy"), None);
    assert!(streamed.body == video, "the video arrived changed");

    // The committed side of a change comes from git, by range as well.
    git(&device.root, &["init", "-q", "-b", "main"]);
    let committed = pattern(5_000);
    std::fs::write(device.root.join("chart.png"), &committed).unwrap();
    git(&device.root, &["add", "chart.png"]);
    git(&device.root, &["commit", "-q", "-m", "chart"]);
    std::fs::write(device.root.join("chart.png"), pattern(7)).unwrap();
    let before = device.get("/changes/raw?path=chart.png").await;
    assert_eq!(before.status, StatusCode::OK);
    assert_eq!(header(&before, "content-type"), Some("image/png"));
    assert_eq!(before.body, committed);
    let tail = device.call(Method::GET, "/changes/raw?path=chart.png", &[("range", "bytes=-100")], None).await;
    assert_eq!(tail.status, StatusCode::PARTIAL_CONTENT);
    assert_eq!(tail.body, committed[committed.len() - 100..]);
    let head = device.call(Method::HEAD, "/changes/raw?path=chart.png", &[], None).await;
    assert_eq!(header(&head, "content-length"), Some("5000"));
    let saved = device.get("/changes/raw?path=chart.png&download=true").await;
    assert_eq!(header(&saved, "content-type"), Some("application/octet-stream"));
    assert!(header(&saved, "content-disposition").unwrap().starts_with("attachment; filename=\"chart.png\""));
    assert_eq!(saved.body, committed);
    assert_eq!(device.get("/changes/raw?path=new.png").await.status, StatusCode::NOT_FOUND);
    let escaping = device.get("/changes/raw?path=../escape.png").await;
    assert_eq!(escaping.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery));

    // Git's copy is read whole, so one over the runner's 8 MiB is refused.
    std::fs::write(device.root.join("poster.png"), pattern(8 * 1024 * 1024 + 1)).unwrap();
    git(&device.root, &["add", "poster.png"]);
    git(&device.root, &["commit", "-q", "-m", "poster"]);
    let oversized = device.get("/changes/raw?path=poster.png").await;
    assert_eq!(oversized.refusal(), (StatusCode::PAYLOAD_TOO_LARGE, ErrorCode::FileTooLarge));
    let head = device.call(Method::HEAD, "/changes/raw?path=poster.png", &[], None).await;
    assert_eq!(head.status, StatusCode::PAYLOAD_TOO_LARGE);
    device.backend.close().await;
}

#[tokio::test]
async fn an_upload_streams_into_place_whole_and_asks_before_it_writes_over_a_file() {
    let device = OnDevice::start().await;
    let put = |path: &str, replace: Option<&str>, body: reqwest::Body| {
        let mut pairs = vec![("path", device.path(path))];
        if let Some(replace) = replace {
            pairs.push(("replace", replace.to_owned()));
        }
        let pairs: Vec<(&str, &str)> = pairs.iter().map(|(name, value)| (*name, value.as_str())).collect();
        let route = format!("/fs/raw?{}", query(&pairs));
        let device = &device;
        async move { device.call(Method::PUT, &route, &[], Some(body)).await }
    };
    let read = |path: &str| std::fs::read_to_string(device.root.join(path)).unwrap();

    assert_eq!(put("notes.md", None, "first".into()).await.status, StatusCode::NO_CONTENT);
    assert_eq!(read("notes.md"), "first");
    let taken = put("notes.md", None, "second".into()).await;
    assert_eq!(taken.refusal(), (StatusCode::CONFLICT, ErrorCode::FileExists));
    assert_eq!(read("notes.md"), "first");
    assert_eq!(put("notes.md", Some("true"), "second".into()).await.status, StatusCode::NO_CONTENT);
    assert_eq!(read("notes.md"), "second");
    assert_eq!(put("empty.txt", None, reqwest::Body::from("")).await.status, StatusCode::NO_CONTENT);
    assert_eq!(std::fs::metadata(device.root.join("empty.txt")).unwrap().len(), 0);

    std::fs::create_dir(device.root.join("docs")).unwrap();
    let directory = put("docs", Some("true"), "x".into()).await;
    assert_eq!(directory.refusal(), (StatusCode::CONFLICT, ErrorCode::IsDirectory));
    let orphan = put("missing/file.txt", None, "x".into()).await;
    assert_eq!(orphan.refusal(), (StatusCode::NOT_FOUND, ErrorCode::FsError));
    let flag = put("notes.md", Some("yes"), "x".into()).await;
    assert_eq!(flag.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery));

    // Far past the JSON body limit and the runner's message limit, streamed
    // through the runner a chunk at a time.
    let block = Bytes::from(pattern(1024 * 1024));
    let chunks = 3 * MAX_MESSAGE_BYTES / block.len() + 1;
    let body = stream::iter((0..chunks).map({
        let block = block.clone();
        move |_| Ok::<_, std::io::Error>(block.clone())
    }));
    let large = put("large.bin", None, reqwest::Body::wrap_stream(body)).await;
    assert_eq!(large.status, StatusCode::NO_CONTENT, "{}", String::from_utf8_lossy(&large.body));
    let written = std::fs::read(device.root.join("large.bin")).unwrap();
    assert_eq!(written.len(), chunks * block.len());
    assert!(written[written.len() - block.len()..] == block[..], "the upload's end arrived changed");

    // A body cut short leaves the path as it was, with no partial copy
    // beside it.
    let broken = stream::iter([Ok::<_, std::io::Error>(block.clone())]).chain(stream::once(async {
        tokio::time::sleep(Duration::from_millis(50)).await;
        Err(std::io::Error::other("the browser went away"))
    }));
    let pairs = [("path", device.path("notes.md")), ("replace", "true".to_owned())];
    let pairs: Vec<(&str, &str)> = pairs.iter().map(|(name, value)| (*name, value.as_str())).collect();
    let url = format!("{}/api/conversations/{CONVERSATION}/fs/raw?{}", device.backend.url, query(&pairs));
    let cut = reqwest::Client::new()
        .put(url)
        .header("cookie", &device.master.cookie)
        .body(reqwest::Body::wrap_stream(broken))
        .send()
        .await;
    assert!(cut.is_err() || cut.unwrap().status() != StatusCode::NO_CONTENT);
    eventually("the cut upload leaves no partial copy", || async {
        std::fs::read_dir(&device.root)
            .unwrap()
            .all(|entry| !entry.unwrap().file_name().to_string_lossy().starts_with(".demi-write-"))
    })
    .await;
    assert_eq!(read("notes.md"), "second");
    device.backend.close().await;
}

#[tokio::test]
async fn a_delete_takes_a_file_or_a_directory_and_refuses_the_directories_the_host_needs() {
    let device = OnDevice::start().await;
    let remove = |path: String| {
        let route = format!("/fs?{}", query(&[("path", &path)]));
        let device = &device;
        async move { device.call(Method::DELETE, &route, &[], None).await }
    };
    std::fs::create_dir_all(device.root.join("photos/2024")).unwrap();
    std::fs::write(device.root.join("photos/2024/a.jpg"), "a").unwrap();
    std::fs::write(device.root.join("notes.md"), "n").unwrap();

    assert_eq!(remove(device.path("notes.md")).await.status, StatusCode::NO_CONTENT);
    assert_eq!(remove(device.path("photos")).await.status, StatusCode::NO_CONTENT);
    assert_eq!(remove(device.path("nothing")).await.status, StatusCode::NO_CONTENT);
    assert_eq!(std::fs::read_dir(&device.root).unwrap().count(), 0);

    let root = device.root.to_str().unwrap().to_owned();
    let home = device.paired.runner.home().to_owned();
    for path in [root.clone(), device.path(".."), home, "/".to_owned(), root.to_uppercase()] {
        let refused = remove(path.clone()).await;
        assert_eq!(refused.refusal(), (StatusCode::CONFLICT, ErrorCode::ProtectedPath), "{path}");
    }
    assert!(device.root.is_dir());
    let relative = remove("photos".to_owned()).await;
    assert_eq!(relative.refusal(), (StatusCode::BAD_REQUEST, ErrorCode::InvalidQuery));
    device.backend.close().await;
}

#[tokio::test]
async fn the_host_access_reaches_only_the_callers_conversation_and_the_hosts_bound_to_it() {
    let device = OnDevice::start().await;
    let backend = &device.backend;

    // Another user's conversation answers as a missing one, as an id that
    // is none does.
    device.harness.add_user("user@example.test", "user-pass-1", Role::User);
    let other = backend.login("user@example.test", "user-pass-1").await;
    let foreign = backend
        .get(&format!("/api/conversations/{CONVERSATION}/fs"), Some(&other))
        .await;
    assert_eq!(foreign.refusal(), (StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound));
    let unknown = backend.get("/api/conversations/not-a-uuid/fs", Some(&device.master)).await;
    assert_eq!(unknown.refusal(), (StatusCode::NOT_FOUND, ErrorCode::ConversationNotFound));

    // A device the conversation does not reach is refused. Attached, it is
    // listed from its home until a shell there ended somewhere.
    let ci = backend.pair(&device.master, "ci").await;
    let on_ci = format!("/hosts/{}/fs", ci.id());
    assert_eq!(device.get(&on_ci).await.refusal(), (StatusCode::NOT_FOUND, ErrorCode::HostNotAttached));
    device.execute(
        "INSERT INTO conversation_hosts (conversation_id, device_id, name, cwd, attached_at)
         VALUES (?1, ?2, 'ci', NULL, 0)",
        &[CONVERSATION, ci.id()],
    );
    let listing = device.get(&on_ci).await;
    assert_eq!(listing.status, StatusCode::OK, "{}", String::from_utf8_lossy(&listing.body));
    let listing: Value = listing.json();
    assert_eq!((&listing["path"], &listing["home"]), (&json!(ci.runner.home()), &json!(ci.runner.home())));
    let made_on_ci = ci.runner.home_dir().join("made");
    let made = device
        .call(Method::POST, &on_ci, &[("content-type", "application/json")], Some(
            json!({ "path": made_on_ci.to_str().unwrap() }).to_string().into(),
        ))
        .await;
    assert_eq!(made.status, StatusCode::CREATED);
    assert!(made_on_ci.is_dir());
    device.execute(
        "UPDATE conversation_hosts SET cwd = ?2 WHERE device_id = ?1",
        &[ci.id(), made_on_ci.to_str().unwrap()],
    );
    let listing: Value = device.get(&on_ci).await.json();
    assert_eq!(listing["path"], json!(made_on_ci.to_str().unwrap()));
    // Named, the main device is the main Host.
    let main: Value = device.get(&format!("/hosts/{}/fs", device.paired.id())).await.json();
    assert_eq!(main["path"], json!(device.root.to_str().unwrap()));

    // An archived conversation refuses every Host operation, transfers too.
    device.execute("UPDATE conversations SET archived = 1 WHERE id = ?1", &[CONVERSATION]);
    let archived = device.get("/fs").await;
    assert_eq!(archived.refusal(), (StatusCode::CONFLICT, ErrorCode::ConversationArchived));
    let raw = format!("/fs/raw?{}", query(&[("path", &device.path("a.txt"))]));
    assert_eq!(device.get(&raw).await.refusal(), (StatusCode::CONFLICT, ErrorCode::ConversationArchived));
    device.backend.close().await;
}

#[tokio::test]
async fn a_shutdown_ends_an_open_download_instead_of_waiting_for_it() {
    let device = OnDevice::start().await;
    std::fs::write(device.root.join("long.mp4"), pattern(64 * 1024 * 1024)).unwrap();
    let route = format!(
        "/api/conversations/{CONVERSATION}/fs/raw?{}",
        query(&[("path", &device.path("long.mp4"))])
    );
    let mut playing = device.backend.response(Method::GET, &route, &device.master, &[], None).await;
    assert_eq!(playing.status(), StatusCode::OK);
    assert!(playing.chunk().await.unwrap().is_some());

    // The page reads nothing more. The shutdown ends the transfer, which
    // holds the conversation's file gate, and goes on.
    let OnDevice { backend, .. } = device;
    tokio::time::timeout(Duration::from_secs(20), backend.close())
        .await
        .expect("the shutdown waited for the download");
    // The download was cut short, never completed.
    let ended = loop {
        match playing.chunk().await {
            Ok(Some(_)) => {}
            Ok(None) => break "complete",
            Err(_) => break "cut",
        }
    };
    assert_eq!(ended, "cut");
}
