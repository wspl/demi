//! Commands outlive their connections and the backend (`runner.md`
//! § Command lifetime, `sessions-and-targets.md` § Recovery and
//! persistence): a connection that drops and comes back within the grace
//! stops nothing, and the command reports its end with what it printed
//! meanwhile; one that stays away loses the command for that reason; a
//! backend that restarts takes its commands up again; and a release change
//! loses them to the upgrade. The model is an Anthropic endpoint the test
//! scripts; the device is a real runner.

use std::net::SocketAddr;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use demi_agent_tools::testing::{field, shown_output};
use demi_host_interface::SpawnEnv;
use demi_provider_common::testing::MockVendor;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_util::task::AbortOnDropHandle;

use crate::conversations::{anthropic_at, create};
use crate::support::{Harness, Session, TestBackend, eventually};
use crate::work::{Driven, resident, say, shell, switch};

const FIRST: &str = "5e2d3c4b-8f3a-4c1e-9d2b-7a1c2e3f4a05";

/// A network between a runner and the backend that can drop every
/// connection it carries, and refuse new ones until it is opened again.
struct Network {
    url: String,
    /// Each change drops every connection.
    cuts: watch::Sender<u64>,
    refusing: Arc<AtomicBool>,
    _accepting: AbortOnDropHandle<()>,
}

impl Network {
    async fn start(backend: SocketAddr) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let cuts = watch::Sender::new(0);
        let refusing = Arc::new(AtomicBool::new(false));
        let (watching, refused) = (cuts.subscribe(), refusing.clone());
        let accepting = tokio::spawn(async move {
            let mut connections = tokio::task::JoinSet::new();
            loop {
                let (client, _) = listener.accept().await.unwrap();
                if refused.load(Ordering::SeqCst) {
                    drop(client);
                    continue;
                }
                connections.spawn(carry(client, backend, watching.clone()));
            }
        });
        Self {
            url,
            cuts,
            refusing,
            _accepting: AbortOnDropHandle::new(accepting),
        }
    }

    /// Drops every connection without a close, as a network that went away
    /// does, and refuses new ones.
    fn cut(&self) {
        self.refusing.store(true, Ordering::SeqCst);
        self.cuts.send_modify(|cut| *cut += 1);
    }

    /// Carries connections again.
    fn open(&self) {
        self.refusing.store(false, Ordering::SeqCst);
    }
}

/// Carries one connection until the network drops it.
async fn carry(mut client: TcpStream, backend: SocketAddr, mut cuts: watch::Receiver<u64>) {
    let Ok(mut server) = TcpStream::connect(backend).await else {
        return;
    };
    cuts.mark_unchanged();
    tokio::select! {
        _ = tokio::io::copy_bidirectional(&mut client, &mut server) => {}
        _ = cuts.changed() => {}
    }
}

/// Waits until something is at `path`.
async fn until_exists(path: &Path) {
    eventually(&format!("{} exists", path.display()), || {
        let exists = path.exists();
        async move { exists }
    })
    .await;
}

/// Waits until a request to the model holds `text`.
async fn until_requested(vendor: &MockVendor, text: &str) {
    eventually(&format!("a request holds {text:?}"), || {
        let held = vendor
            .requests()
            .iter()
            .any(|request| request.json().to_string().contains(text));
        async move { held }
    })
    .await;
}

/// The jobs the control database records running.
fn running_jobs(harness: &Harness) -> i64 {
    harness
        .control_database()
        .query_row("SELECT count(*) FROM running_jobs", [], |row| row.get(0))
        .unwrap()
}

/// A command that prints, waits for `go`, prints while the test may keep
/// its runner away, waits for `finish`, prints again and exits with 3.
const WAITING: &str = "printf 'before\\n'; while [ ! -f go ]; do sleep 0.05; done; printf 'during\\n'; touch printed; while [ ! -f finish ]; do sleep 0.05; done; printf 'after\\n'; exit 3";

/// Starts `script` in the conversation with the call `call` as a command
/// that reports only its end, whose call returns while it runs once its
/// output is quiet, and answers its number.
async fn started(work: &mut Driven<'_>, call: &str, script: &str) -> String {
    let started = work
        .turn(vec![resident(call, script), say("waiting")])
        .await;
    let result = &started.received[0];
    assert!(result.starts_with("status: running"), "{result}");
    field(result, "commandId").to_owned()
}

/// A conversation on a device paired through `network`, opened.
async fn on_device<'a>(
    backend: &TestBackend,
    master: &Session,
    vendor: &'a MockVendor,
    paired: &crate::support::Paired,
) -> Driven<'a> {
    let provider = anthropic_at(backend, master, vendor, "/lifetime").await;
    create(backend, master, FIRST).await;
    let home = paired.runner.home_dir().to_owned();
    switch(backend, master, FIRST, paired, &home).await;
    Driven::open(backend, master, vendor, FIRST, &provider, "/lifetime").await
}

// About four seconds: a real device pairs, its job is a login shell, and
// the call that starts it waits for two quiet seconds.
#[tokio::test]
async fn a_connection_that_comes_back_within_the_grace_stops_nothing_and_the_end_reports_what_was_printed_meanwhile()
 {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let network = Network::start(backend.address()).await;
    let alpha = backend.pair_through(&master, "alpha", &network.url).await;
    let mut work = on_device(&backend, &master, &vendor, &alpha).await;
    let home = alpha.runner.home_dir().to_owned();
    let command = started(&mut work, "t1", WAITING).await;
    assert_eq!(running_jobs(&harness), 1);

    // The network goes away; the command prints meanwhile.
    network.cut();
    backend.until_online(&master, alpha.id(), false).await;
    std::fs::write(home.join("go"), "").unwrap();
    until_exists(&home.join("printed")).await;
    network.open();
    backend.until_online(&master, alpha.id(), true).await;

    // The command goes on there, and its end wakes the model with what it
    // printed, the output from while it was away among it.
    work.script(vec![say("noted")]);
    std::fs::write(home.join("finish"), "").unwrap();
    until_requested(&vendor, &format!("Command {command} (t1) ended with exit code 3.")).await;
    work.socket.until_idle().await;
    let read = format!("demi shell output {command} --raw");
    let shown = work.turn(vec![shell("t2", &read, 10_000), say("read")]).await;
    assert_eq!(shown_output(&shown.received[0]), "before\nduring\nafter\n");
    assert_eq!(running_jobs(&harness), 0, "the job's record goes with its end");
    backend.close().await;
}

// About four seconds: a real device pairs, its job is a login shell, the
// call that starts it waits for two quiet seconds, and its runner keeps the
// job 300 ms instead of 10 minutes.
#[tokio::test]
async fn a_connection_that_stays_away_past_the_grace_loses_the_command_and_says_why() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let network = Network::start(backend.address()).await;
    let grace = SpawnEnv::Overlay([("DEMI_UNREACHED_GRACE_MS".to_owned(), Some("300".to_owned()))].into());
    let alpha = backend.pair_with(&master, "alpha", &network.url, grace).await;
    let mut work = on_device(&backend, &master, &vendor, &alpha).await;
    let home = alpha.runner.home_dir().to_owned();
    let command = started(&mut work, "t1", "sh -c 'echo $$ > pid; exec sleep 60'").await;
    until_exists(&home.join("pid")).await;
    let pid = std::fs::read_to_string(home.join("pid")).unwrap().trim().to_owned();
    let alive = |pid: &str| {
        std::process::Command::new("kill")
            .args(["-0", pid])
            .stderr(std::process::Stdio::null())
            .status()
            .unwrap()
            .success()
    };

    network.cut();
    eventually("the runner stopped its job", || {
        let alive = alive(&pid);
        async move { !alive }
    })
    .await;
    work.script(vec![say("noted")]);
    network.open();

    until_requested(
        &vendor,
        &format!("Command {command} (t1) was lost: {}. Start it again", demi_backend_remote_host::UNREACHED),
    )
    .await;
    work.socket.until_idle().await;
    let look = format!("demi shell status {command}");
    let shown = work.turn(vec![shell("t2", &look, 10_000), say("looked")]).await;
    assert!(
        shown.received[0].contains(&format!("status: lost: {}", demi_backend_remote_host::UNREACHED)),
        "{}",
        shown.received[0]
    );
    backend.close().await;
}

// Several seconds: a real device pairs, two calls each wait for two quiet
// seconds, the backend restarts, and the runner comes back to it.
#[tokio::test]
async fn a_backend_restart_takes_running_commands_up_again_and_they_report_their_ends() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let mut work = on_device(&backend, &master, &vendor, &alpha).await;
    let home = alpha.runner.home_dir().to_owned();
    std::fs::write(home.join("go"), "").unwrap();
    let going_on = started(&mut work, "t1", WAITING).await;
    let ending = started(
        &mut work,
        "t2",
        "printf 'waiting\\n'; while [ ! -f stop ]; do sleep 0.05; done; touch stopped; exit 5",
    )
    .await;

    // The backend restarts at its address, where the runner reconnects; one
    // command ends meanwhile, the other keeps running on the Host.
    let address = backend.address();
    backend.close().await;
    assert_eq!(running_jobs(&harness), 2, "the jobs' records survive the restart");
    std::fs::write(home.join("stop"), "").unwrap();
    until_exists(&home.join("stopped")).await;
    work.script(vec![say("noted"), say("noted")]);
    let backend = harness.start_at(address).await;
    backend.until_online(&master, alpha.id(), true).await;
    until_requested(&vendor, &format!("Command {ending} (t2) ended with exit code 5.")).await;
    until_exists(&home.join("printed")).await;
    std::fs::write(home.join("finish"), "").unwrap();

    until_requested(&vendor, &format!("Command {going_on} (t1) ended with exit code 3.")).await;
    eventually("the jobs' records go with their ends", || {
        let running = running_jobs(&harness);
        async move { running == 0 }
    })
    .await;
    backend.close().await;
}

// Several seconds: a real device pairs, the backend restarts, and the
// device's runner starts anew as another release's would.
#[tokio::test]
async fn a_release_change_loses_running_commands_to_the_upgrade() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut alpha = backend.pair(&master, "alpha").await;
    let mut work = on_device(&backend, &master, &vendor, &alpha).await;
    let command = started(&mut work, "t1", "sleep 60").await;

    // The upgrade stops the backend, and the device's runner replaces itself
    // with the new release's: the device last ran another release.
    let address = backend.address();
    backend.close().await;
    alpha.runner.kill().await;
    harness
        .control_database()
        .execute("UPDATE devices SET runner_version = '0.0.1'", [])
        .unwrap();
    work.script(vec![say("noted")]);
    let backend = harness.start_at(address).await;
    alpha.runner.start_again();
    backend.until_online(&master, alpha.id(), true).await;

    until_requested(
        &vendor,
        &format!(
            "Command {command} (t1) was lost: {}. Start it again if it is still needed.",
            demi_backend_remote_host::UPGRADED
        ),
    )
    .await;
    backend.close().await;
}
