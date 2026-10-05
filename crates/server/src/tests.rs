//! Moves between releases on a server laid out in a directory, with the
//! services simulated (`upgrades.md` § Acceptance): each release's programs
//! are scripts that accept the configuration and the import, its
//! `demi-server` names a gVisor version of its own, and a release named as
//! failing does not start.

use std::{
    cell::RefCell,
    io,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
};

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
/// database of a schema 0.2.0 migrates.
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
            .execute_batch("CREATE TABLE users (id TEXT); INSERT INTO users VALUES ('a'); PRAGMA user_version = 7;")
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
        control
            .execute_batch("INSERT INTO users VALUES ('since'); PRAGMA user_version = 8;")
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
    assert_eq!(fetch::newest(&source).await.unwrap(), version);
    fetch::fetch(&layout, &source, &version, &cancel).await.unwrap();
    let release = layout.release(&version);
    assert!(release.join("bin/demi-server").exists());
    assert!(release.join("image/manifest.json").exists());
    assert_eq!(layout.versions().unwrap(), [version]);
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
