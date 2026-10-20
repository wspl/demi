//! Moves between releases on a server laid out in a directory, with the
//! services simulated: each release's programs are scripts that accept the
//! configuration and the import, its `demi-server` names a gVisor version
//! of its own, and a release named as failing does not start.

use std::{
    cell::RefCell,
    io,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
};

use demi_backend_database::{DatabaseKind, schema_testing};
use semver::Version;
use tokio_util::sync::CancellationToken;

use crate::{
    fetch,
    journal::{Data, Journal, Step},
    layout::{Layout, UNITS},
    moving::{self, Failure},
    services::Services,
};

/// Services that start unless the release `current` names is failing.
struct Simulated<'a> {
    layout: &'a Layout,
    failing: Vec<Version>,
    calls: RefCell<Vec<String>>,
}

impl Services for Simulated<'_> {
    fn stop(&self, unit: &str) -> io::Result<()> {
        self.calls.borrow_mut().push(format!("stop {unit}"));
        Ok(())
    }

    fn start(&self, unit: &str) -> io::Result<()> {
        let version = self.layout.current_version()?;
        self.calls.borrow_mut().push(format!("start {unit} {version}"));
        if self.failing.contains(&version) {
            return Err(io::Error::other("it exited"));
        }
        Ok(())
    }

    fn reload(&self) -> io::Result<()> {
        self.calls.borrow_mut().push("reload".into());
        Ok(())
    }

    fn log(&self, unit: &str) -> String {
        format!("{unit}: exited")
    }
}

/// A server running 0.1.0 with 0.2.0 unpacked beside it, and a control
/// database of the schema the last published release shipped, which 0.2.0,
/// this build, migrates.
struct Server {
    root: tempfile::TempDir,
    layout: Layout,
}

const OLD: Version = Version::new(0, 1, 0);
const NEW: Version = Version::new(0, 2, 0);

impl Server {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let layout = Layout::new(root.path().to_owned());
        for version in [OLD, NEW] {
            release(&layout, &version);
        }
        std::fs::create_dir_all(layout.current().parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(layout.release(&OLD), layout.current()).unwrap();
        let server = Self { root, layout };
        let config = server.layout.config();
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        std::fs::write(
            &config,
            format!(
                "DEMI_BACKEND_DATA={}\nDEMI_MANAGED_DATA={}\n",
                server.backend().display(),
                server.manager().display()
            ),
        )
        .unwrap();
        std::fs::create_dir_all(server.manager()).unwrap();
        let control = rusqlite::Connection::open(server.backend().join("control.sqlite")).unwrap();
        control
            .execute_batch("CREATE TABLE users (id TEXT); INSERT INTO users VALUES ('a');")
            .unwrap();
        control
            .pragma_update(None, "user_version", schema_testing::shipped_version(DatabaseKind::Control))
            .unwrap();
        server
    }

    fn backend(&self) -> PathBuf {
        let backend = self.root.path().join("data/backend");
        std::fs::create_dir_all(&backend).unwrap();
        backend
    }

    fn manager(&self) -> PathBuf {
        self.root.path().join("data/manager")
    }

    fn services(&self, failing: &[Version]) -> Simulated<'_> {
        Simulated {
            layout: &self.layout,
            failing: failing.to_vec(),
            calls: RefCell::default(),
        }
    }

    fn users(&self) -> Vec<String> {
        let control = rusqlite::Connection::open(self.backend().join("control.sqlite")).unwrap();
        let mut statement = control.prepare("SELECT id FROM users").unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect()
    }

    /// What the new backend would leave: a migrated database.
    fn migrate(&self) {
        let control = rusqlite::Connection::open(self.backend().join("control.sqlite")).unwrap();
        control.execute_batch("INSERT INTO users VALUES ('since');").unwrap();
        control
            .pragma_update(None, "user_version", schema_testing::current_version(DatabaseKind::Control))
            .unwrap();
    }

    fn unit(&self, unit: &str) -> String {
        std::fs::read_to_string(self.layout.units().join(unit)).unwrap()
    }
}

/// A release root with programs that accept every check, a `demi-server`
/// whose `runtime` makes and names the release's gVisor version,
/// `gvisor-<version>`, and its units, which name its version.
fn release(layout: &Layout, version: &Version) {
    let root = layout.release(version);
    std::fs::create_dir_all(root.join("bin")).unwrap();
    std::fs::create_dir_all(root.join("systemd")).unwrap();
    let gvisor = layout.gvisor().join(format!("gvisor-{version}"));
    let runtime = format!("#!/bin/sh\nmkdir -p '{0}'\necho '{0}'\n", gvisor.display());
    for (program, script) in [
        ("demi-backend", "#!/bin/sh\nexit 0\n"),
        ("demi-machine-manager", "#!/bin/sh\nexit 0\n"),
        ("demi-server", runtime.as_str()),
    ] {
        let path = root.join("bin").join(program);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    for unit in UNITS {
        std::fs::write(root.join("systemd").join(unit), format!("{unit} of {version}\n")).unwrap();
    }
}

#[test]
fn an_upgrade_copies_what_it_migrates_and_its_rollback_puts_it_back() {
    let server = Server::new();
    // A version an earlier release used, and a stage a killed fetch left.
    for left in ["gvisor-0.0.1", ".staging-x"] {
        std::fs::create_dir_all(server.layout.gvisor().join(left)).unwrap();
    }
    let services = server.services(&[]);
    moving::start(&server.layout, &services, &NEW).unwrap();
    let mut gvisor: Vec<String> = std::fs::read_dir(server.layout.gvisor())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    gvisor.sort();
    assert_eq!(gvisor, ["gvisor-0.1.0", "gvisor-0.2.0"]);
    assert_eq!(server.layout.current_version().unwrap(), NEW);
    assert_eq!(server.unit(UNITS[1]), format!("{} of {NEW}\n", UNITS[1]));
    assert_eq!(
        *services.calls.borrow(),
        [
            "stop demi-backend.service",
            "stop demi-machine-manager.service",
            "reload",
            "start demi-machine-manager.service 0.2.0",
            "start demi-backend.service 0.2.0",
        ]
    );
    assert!(server.backend().join("snapshots/0.1.0/control.sqlite").exists());
    assert!(!server.layout.journal().exists());

    // The new release writes, and the server goes back to the old one.
    server.migrate();
    let services = server.services(&[]);
    moving::start(&server.layout, &services, &OLD).unwrap();
    assert_eq!(server.layout.current_version().unwrap(), OLD);
    assert_eq!(server.unit(UNITS[1]), format!("{} of {OLD}\n", UNITS[1]));
    assert_eq!(server.users(), ["a"]);
    // What the new release wrote is set aside, not deleted.
    let abandoned = server.backend().join("snapshots/abandoned-0.2.0/control.sqlite");
    let aside = rusqlite::Connection::open(abandoned).unwrap();
    let count: i64 = aside.query_row("SELECT count(*) FROM users", [], |row| row.get(0)).unwrap();
    assert_eq!(count, 2);
    assert!(!server.backend().join("snapshots/0.1.0").exists());
}

#[test]
fn a_release_that_does_not_start_returns_the_server_with_its_data() {
    let server = Server::new();
    let services = server.services(&[NEW]);
    let failed = moving::start(&server.layout, &services, &NEW);
    assert!(
        matches!(&failed, Err(Failure::Returned { log, .. }) if log.contains("exited")),
        "{failed:?}"
    );
    assert_eq!(server.layout.current_version().unwrap(), OLD);
    assert_eq!(server.unit(UNITS[0]), format!("{} of {OLD}\n", UNITS[0]));
    assert_eq!(server.users(), ["a"]);
    assert!(services.calls.borrow().ends_with(&[
        "start demi-machine-manager.service 0.1.0".to_owned(),
        "start demi-backend.service 0.1.0".to_owned(),
    ]));
    assert!(!server.layout.journal().exists());
}

/// A database the new release could not open, such as one a development
/// build of another unpublished schema made, stops the upgrade before
/// either service stops, naming the file: the control database and a
/// conversation's alike.
#[test]
fn an_upgrade_refuses_a_database_its_release_cannot_open_before_anything_stops() {
    for database in ["control.sqlite", "conversations/c1.sqlite"] {
        let server = Server::new();
        let path = server.backend().join(database);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let other = rusqlite::Connection::open(&path).unwrap();
        other.execute_batch("CREATE TABLE IF NOT EXISTS notes (id TEXT);").unwrap();
        // The version of a schema this build neither has nor migrates from.
        other.pragma_update(None, "user_version", 7).unwrap();
        drop(other);
        let services = server.services(&[]);
        let refused = moving::start(&server.layout, &services, &NEW);
        assert!(
            matches!(&refused, Err(Failure::Failed(error)) if error.to_string().contains(&path.display().to_string())),
            "{database}: {refused:?}"
        );
        assert!(services.calls.borrow().is_empty(), "{database}");
        assert_eq!(server.layout.current_version().unwrap(), OLD, "{database}");
        assert!(!server.layout.journal().exists(), "{database}");
        assert!(!server.backend().join("snapshots").exists(), "{database}");
    }
}

#[test]
fn a_move_interrupted_at_any_step_is_finished_by_the_next_run() {
    for step in [Step::Stopping, Step::Data, Step::Switching, Step::Starting] {
        let server = Server::new();
        // The run that recorded `step` ended before taking it.
        let mut journal = Journal {
            from: OLD,
            to: NEW,
            data: Data::Snapshot,
            step,
        };
        if step > Step::Data {
            std::fs::create_dir_all(server.backend().join("snapshots/0.1.0")).unwrap();
            std::fs::copy(
                server.backend().join("control.sqlite"),
                server.backend().join("snapshots/0.1.0/control.sqlite"),
            )
            .unwrap();
        }
        if step > Step::Switching {
            server.layout.point_at(&NEW).unwrap();
            server.layout.install_units(&NEW).unwrap();
        }
        journal.record(&server.layout.journal(), step).unwrap();
        let services = server.services(&[]);
        moving::carry_out(&server.layout, &services, journal).unwrap();
        assert_eq!(server.layout.current_version().unwrap(), NEW, "{step:?}");
        assert_eq!(server.unit(UNITS[1]), format!("{} of {NEW}\n", UNITS[1]), "{step:?}");
        assert!(server.backend().join("snapshots/0.1.0/control.sqlite").exists(), "{step:?}");
        assert!(!server.layout.journal().exists(), "{step:?}");
    }
}

#[test]
fn a_return_that_does_not_finish_leaves_the_upgrade_for_the_next_run_to_finish() {
    let server = Server::new();
    // The new release does not start, and the old one does not start again
    // either.
    let services = server.services(&[OLD, NEW]);
    let failed = moving::start(&server.layout, &services, &NEW);
    assert!(
        matches!(&failed, Err(Failure::Failed(error)) if error.to_string().contains("did not start again either")),
        "{failed:?}"
    );
    assert_eq!(server.layout.current_version().unwrap(), OLD);

    // Once the services start, the next run ends at the release it reports:
    // the one the journal's move goes to.
    let journal = Journal::read(&server.layout.journal()).unwrap().unwrap();
    let to = journal.to.clone();
    let services = server.services(&[]);
    moving::carry_out(&server.layout, &services, journal).unwrap();
    assert_eq!(server.layout.current_version().unwrap(), to);
    assert_eq!(server.unit(UNITS[1]), format!("{} of {to}\n", UNITS[1]));
    assert!(!server.layout.journal().exists());
}

/// A release's assets as the release workflow publishes them, in a
/// directory.
fn assets(directory: &Path, version: &Version, tamper: bool) {
    let architecture = fetch::architecture().unwrap();
    let program = b"#!/bin/sh\nexit 0\n";
    let mut server = tar::Builder::new(Vec::new());
    let mut header = tar::Header::new_gnu();
    header.set_size(program.len() as u64);
    header.set_mode(0o755);
    server.append_data(&mut header, "./bin/demi-server", &program[..]).unwrap();
    let server = zstd::encode_all(&server.into_inner().unwrap()[..], 0).unwrap();
    let mut image = tar::Builder::new(Vec::new());
    let manifest = b"{}";
    let mut header = tar::Header::new_gnu();
    header.set_size(manifest.len() as u64);
    header.set_mode(0o644);
    image.append_data(&mut header, "image/manifest.json", &manifest[..]).unwrap();
    let image = image.into_inner().unwrap();
    let names = [
        format!("demi-{version}-server-linux-{architecture}.tar.zst"),
        format!("demi-{version}-image-linux-{architecture}.tar"),
    ];
    let mut sums = String::new();
    for (name, bytes) in names.iter().zip([&server, &image]) {
        use sha2::Digest as _;
        sums.push_str(&format!("{:x}  {name}\n", sha2::Sha256::digest(bytes)));
        std::fs::write(directory.join(name), bytes).unwrap();
    }
    if tamper {
        std::fs::write(directory.join(&names[1]), b"another image").unwrap();
    }
    std::fs::write(directory.join("SHA256SUMS"), sums).unwrap();
}

#[tokio::test]
async fn a_release_is_fetched_whole_only_when_its_assets_match_their_sums() {
    let cancel = CancellationToken::new();
    let root = tempfile::tempdir().unwrap();
    let layout = Layout::new(root.path().to_owned());
    let version = Version::new(0, 3, 0);
    let tampered = tempfile::tempdir().unwrap();
    assets(tampered.path(), &version, true);
    let source = fetch::Source::Directory(tampered.path().to_owned());
    let refused = fetch::fetch(&layout, &source, &version, &cancel).await;
    assert!(refused.is_err());
    assert!(!layout.release(&version).exists());

    let published = tempfile::tempdir().unwrap();
    assets(published.path(), &version, false);
    let source = fetch::Source::Directory(published.path().to_owned());
    assert_eq!(fetch::newest(&source, false).await.unwrap(), version);
    fetch::fetch(&layout, &source, &version, &cancel).await.unwrap();
    let release = layout.release(&version);
    assert!(release.join("bin/demi-server").exists());
    assert!(release.join("image/manifest.json").exists());
    assert_eq!(layout.versions().unwrap(), [version]);
}

#[test]
fn the_newest_release_is_the_highest_version_tagged_v_pre_releases_included() {
    // As GitHub lists them: a deploy's pre-release above the last published
    // release, the gVisor build's own tag, and 0.1.10 after 0.1.9 in
    // version order, not in the order of the strings.
    let tags = ["runsc-release-20260901.0", "v0.1.22", "v0.1.21", "v0.1.9", "v0.1.10", "nightly"];
    assert_eq!(fetch::highest(tags), Some(Version::new(0, 1, 22)));
    assert_eq!(fetch::highest(["v0.1.9", "v0.1.10"]), Some(Version::new(0, 1, 10)));
    assert_eq!(fetch::highest(["runsc-release-20260901.0"]), None);
}

#[test]
fn prerelease_is_refused_beside_a_version() {
    #[derive(clap::Parser)]
    struct Command {
        #[command(flatten)]
        options: crate::setup::Options,
    }
    let refused = <Command as clap::Parser>::try_parse_from(["setup", "--prerelease", "--version", "0.1.22"]);
    assert_eq!(refused.err().map(|error| error.kind()), Some(clap::error::ErrorKind::ArgumentConflict));
    let refused = <crate::Cli as clap::Parser>::try_parse_from(["demi-server", "upgrade", "--prerelease", "0.1.22"]);
    assert_eq!(refused.err().map(|error| error.kind()), Some(clap::error::ErrorKind::ArgumentConflict));
}

/// `setup`'s options as a command line gives them.
fn setup_options(args: &[&str]) -> crate::setup::Options {
    #[derive(clap::Parser)]
    struct Command {
        #[command(flatten)]
        options: crate::setup::Options,
    }
    let mut line = vec!["setup"];
    line.extend(args);
    <Command as clap::Parser>::parse_from(line).options
}

#[test]
fn setup_without_its_choices_and_without_asking_answers_with_the_guide_and_changes_nothing() {
    let root = tempfile::tempdir().unwrap();
    let layout = Layout::new(root.path().to_owned());
    let services = Simulated {
        layout: &layout,
        failing: Vec::new(),
        calls: RefCell::default(),
    };
    let outcome = crate::setup::run(&layout, &services, setup_options(&["--domain", "demi.example.com", "--no-input"]));
    assert!(matches!(outcome, Ok(crate::setup::Outcome::Guide)));
    assert_eq!(std::fs::read_dir(root.path()).unwrap().count(), 0);
    assert!(services.calls.borrow().is_empty());
}

#[test]
fn setup_refuses_a_server_that_is_set_up_and_an_address_for_a_domain() {
    let server = Server::new();
    let services = server.services(&[]);
    let refused = crate::setup::run(
        &server.layout,
        &services,
        setup_options(&["--domain", "demi.example.com", "--mode", "isolated", "--listen", "127.0.0.1:3271", "--no-input"]),
    );
    assert!(refused.unwrap_err().to_string().contains("demi-server upgrade"));

    let root = tempfile::tempdir().unwrap();
    let layout = Layout::new(root.path().to_owned());
    let refused = crate::setup::run(
        &layout,
        &services,
        setup_options(&["--domain", "203.0.113.7", "--mode", "isolated", "--listen", "127.0.0.1:3271", "--no-input"]),
    );
    assert!(refused.unwrap_err().to_string().contains("no domain"));
    assert!(services.calls.borrow().is_empty());
}

/// A release root whose `runners/` names one runner release, and the
/// management socket of a backend, both in `directory`; the backend's
/// side of each connection is `script`'s, run on a thread of its own with
/// the request it read.
fn runners_step(
    directory: &Path,
    script: impl FnOnce(String, std::os::unix::net::UnixStream) + Send + 'static,
) -> (PathBuf, PathBuf, std::thread::JoinHandle<()>) {
    use std::io::BufRead as _;
    let root = directory.join("root");
    std::fs::create_dir_all(root.join("runners")).unwrap();
    let record = serde_json::json!({
        "release": "3".repeat(64),
        "wire": demi_runner_protocol::wire::VERSION,
        "commandProtocol": demi_command_protocol::VERSION,
        "targets": { "x86_64-unknown-linux-musl": { "sha256": "4".repeat(64), "size": 1 } },
    });
    std::fs::write(root.join("runners/manifest.json"), record.to_string()).unwrap();
    let socket = directory.join("manage.sock");
    let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
    let backend = std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        std::io::BufReader::new(&stream).read_line(&mut request).unwrap();
        script(request, stream);
    });
    (root, socket, backend)
}

/// Writes each of `lines`, a backend's answer, to `stream`.
fn answer(mut stream: &std::os::unix::net::UnixStream, lines: &[serde_json::Value]) {
    use std::io::Write as _;
    for line in lines {
        writeln!(stream, "{line}").unwrap();
    }
}

/// The runners step names the next runner release and its root to the
/// running backend, prints how many devices have it as they finish, and
/// goes on once none is downloading, naming those that will download
/// after the switch.
#[test]
fn the_runners_step_waits_for_the_devices_and_says_how_many_have_the_new_runner() {
    let directory = tempfile::tempdir().unwrap();
    let (root, socket, backend) = runners_step(directory.path(), |request, stream| {
        let request: serde_json::Value = serde_json::from_str(&request).unwrap();
        assert_eq!(request["release"], "3".repeat(64));
        answer(&stream, &[
            serde_json::json!({ "devices": [
                { "name": "laptop", "state": "downloading" },
                { "name": "hk-server", "state": "downloading" },
            ] }),
            serde_json::json!({ "devices": [
                { "name": "laptop", "state": "done" },
                { "name": "hk-server", "state": "downloading" },
            ] }),
            serde_json::json!({ "devices": [
                { "name": "laptop", "state": "done" },
                { "name": "hk-server", "state": "failed", "reason": "the connection ended" },
            ] }),
        ]);
    });
    let mut out = Vec::new();
    crate::runners::prefetch(&socket, &root, crate::runners::WAIT, &mut out).unwrap();
    backend.join().unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "0 of 2 devices have the new runner\n\
         1 of 2 devices have the new runner\n\
         hk-server downloads the new runner after the switch: the connection ended\n"
    );
}

/// The runners step waits no longer than its limit: it goes on, naming the
/// devices still downloading, which download after the switch.
#[test]
fn the_runners_step_goes_on_at_its_limit() {
    let directory = tempfile::tempdir().unwrap();
    let (gone_on, going_on) = std::sync::mpsc::channel::<()>();
    let (root, socket, backend) = runners_step(directory.path(), move |_, stream| {
        answer(&stream, &[serde_json::json!({ "devices": [
            { "name": "laptop", "state": "done" },
            { "name": "hk-server", "state": "downloading" },
        ] })]);
        // The download goes on until the step went on.
        let _ = going_on.recv();
    });
    let mut out = Vec::new();
    let limit = std::time::Duration::from_millis(300);
    let started = std::time::Instant::now();
    crate::runners::prefetch(&socket, &root, limit, &mut out).unwrap();
    assert!(started.elapsed() >= limit, "{:?}", started.elapsed());
    drop(gone_on);
    backend.join().unwrap();
    assert_eq!(
        String::from_utf8(out).unwrap(),
        "1 of 2 devices have the new runner\n\
         hk-server has not finished and downloads the new runner after the switch\n"
    );
}

/// A backend of a release without the management socket is told nothing.
#[test]
fn the_runners_step_tells_a_backend_without_the_socket_nothing() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path().join("root");
    std::fs::create_dir_all(root.join("runners")).unwrap();
    std::fs::write(
        root.join("runners/manifest.json"),
        serde_json::json!({
            "release": "3".repeat(64),
            "wire": demi_runner_protocol::wire::VERSION,
            "commandProtocol": demi_command_protocol::VERSION,
            "targets": { "x86_64-unknown-linux-musl": { "sha256": "4".repeat(64), "size": 1 } },
        })
        .to_string(),
    )
    .unwrap();
    let mut out = Vec::new();
    let socket = directory.path().join("manage.sock");
    crate::runners::prefetch(&socket, &root, crate::runners::WAIT, &mut out).unwrap();
    assert!(out.is_empty());
}
