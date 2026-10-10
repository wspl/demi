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
use demi_conversation_socket_protocol::ServerFrame;
use demi_host_interface::SpawnEnv;
use demi_provider_common::testing::MockVendor;
use demi_shared_types::{Block, ReportEvent, SessionPhase};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::watch;
use tokio_util::task::AbortOnDropHandle;

use crate::conversations::{anthropic_at, create, transcript};
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

/// How many requests to the model hold `text`.
fn requested(vendor: &MockVendor, text: &str) -> usize {
    vendor
        .requests()
        .iter()
        .filter(|request| request.json().to_string().contains(text))
        .count()
}

/// The command reports the conversation's transcript holds, in order.
async fn wakeups(backend: &TestBackend, master: &Session) -> Vec<demi_shared_types::CommandReport> {
    transcript(backend, master, FIRST)
        .await
        .blocks
        .into_iter()
        .filter_map(|block| match block {
            Block::Wakeup(wakeup) => Some(wakeup.reports),
            _ => None,
        })
        .flatten()
        .collect()
}

/// How many reports of a loss wait in the root's saved checkpoint for the
/// turn to take them.
fn lost_reports_waiting(harness: &Harness) -> usize {
    let file = harness.data_dir().join("conversations").join(format!("{FIRST}.sqlite"));
    let connection = rusqlite::Connection::open(file).unwrap();
    connection.busy_timeout(std::time::Duration::from_secs(5)).unwrap();
    let state: String = connection
        .query_row("SELECT state FROM nodes WHERE parent_id IS NULL", [], |row| row.get(0))
        .unwrap();
    let state: serde_json::Value = serde_json::from_str(&state).unwrap();
    state["reports"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|report| report["event"]["kind"] == "lost")
        .count()
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

// About three seconds: a real device pairs, and the call that starts the
// command waits for two quiet seconds.
#[tokio::test]
async fn a_command_that_ends_while_its_conversation_is_closed_reports_its_end_when_it_opens() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let mut work = on_device(&backend, &master, &vendor, &alpha).await;
    let home = alpha.runner.home_dir().to_owned();
    let script = "printf 'started\\n'; while [ ! -f finish ]; do sleep 0.05; done; printf 'ended\\n'; touch done; exit 3";
    let command = started(&mut work, "t1", script).await;

    // The page closes the conversation, whose command runs on, and ends.
    work.socket
        .send(&demi_conversation_socket_protocol::ClientFrame::Close {})
        .await;
    work.socket
        .until(|frame| matches!(frame, demi_conversation_socket_protocol::ServerFrame::Closed))
        .await;
    std::fs::write(home.join("finish"), "").unwrap();
    until_exists(&home.join("done")).await;
    assert_eq!(running_jobs(&harness), 1, "no agent recorded its end");

    // Opened again, the agent takes the command up and hears of its end.
    work.script(vec![say("noted")]);
    let provider = anthropic_at(&backend, &master, &vendor, "/lifetime").await;
    work.reconnect(&backend, &master, FIRST, &provider).await;
    until_requested(&vendor, &format!("Command {command} (t1) ended with exit code 3.")).await;
    eventually("the job's record goes with its end", || {
        let running = running_jobs(&harness);
        async move { running == 0 }
    })
    .await;
    backend.close().await;
}

// Several seconds: a real device pairs, the call waits for two quiet
// seconds, and the backend restarts while the runner is paused.
#[tokio::test]
async fn an_rpc_call_of_a_job_no_agent_took_up_yet_takes_it_up_and_is_served() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let mut work = on_device(&backend, &master, &vendor, &alpha).await;
    let home = alpha.runner.home_dir().to_owned();
    // Once let go, the job asks the backend about itself with an rpc call.
    let script = "printf 'started\\n'; while [ ! -f go ]; do sleep 0.05; done; demi shell status 1 > status.txt 2>&1; touch asked; while [ ! -f finish ]; do sleep 0.05; done";
    let command = started(&mut work, "t1", script).await;
    assert_eq!(command, "1");

    // The backend restarts, and holds the take-up its runner's hello asks
    // for: the job is known only from its records when it calls.
    let address = backend.address();
    backend.close().await;
    let backend = harness.start_at(address).await;
    let takes_up = backend.hold_hellos(demi_backend_user_shard::holds::HelloStep::TakeUp);
    backend.until_online(&master, alpha.id(), true).await;
    std::fs::write(home.join("go"), "").unwrap();
    until_exists(&home.join("asked")).await;
    let status = std::fs::read_to_string(home.join("status.txt")).unwrap();
    assert!(status.starts_with("status: running"), "{status}");

    takes_up.release();
    std::fs::write(home.join("finish"), "").unwrap();
    backend.close().await;
}

// Several seconds: a real device pairs and installs the builtin package,
// the call waits for two quiet seconds, and the network goes away and
// comes back.
#[tokio::test]
async fn a_medium_a_job_returns_while_its_connection_is_away_reaches_its_end_result() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new().with_file_package();
    let (backend, master) = harness.start_set_up().await;
    let network = Network::start(backend.address()).await;
    let alpha = backend.pair_through(&master, "alpha", &network.url).await;
    let mut work = on_device(&backend, &master, &vendor, &alpha).await;
    let home = alpha.runner.home_dir().to_owned();
    let png = demi_agent_store::testing::png(4, 3, 1).into_bytes();
    std::fs::write(home.join("shot.png"), &png).unwrap();
    // The package is installed while the runner is connected.
    work.turn(vec![shell("t0", "demi file view shot.png > /dev/null", 30_000), say("viewed")])
        .await;
    let script = "printf 'waiting\\n'; while [ ! -f go ]; do sleep 0.05; done; demi file view shot.png > /dev/null 2> view.err; touch viewed";
    let command = started(&mut work, "t1", script).await;

    // The job returns its medium while its runner is away, and ends.
    network.cut();
    backend.until_online(&master, alpha.id(), false).await;
    std::fs::write(home.join("go"), "").unwrap();
    until_exists(&home.join("viewed")).await;
    work.script(vec![say("noted")]);
    network.open();
    until_requested(&vendor, &format!("Command {command} (t1) ended with exit code 0.")).await;
    work.socket.until_idle().await;

    // Its end report carries the medium, which the next connection
    // announced again (`runtime.md` § What a result attaches).
    let report = format!("Command {command} (t1) ended with exit code 0.");
    let message = vendor
        .requests()
        .iter()
        .filter_map(|request| {
            request.json()["messages"]
                .as_array()?
                .iter()
                .find(|message| message.to_string().contains(&report))
                .cloned()
        })
        .next()
        .expect("a request carries the report");
    let carried = message.to_string();
    assert!(carried.contains("\"type\":\"image\""), "{carried}");
    backend.close().await;
}

// Several seconds: a real device pairs, the call waits for two quiet
// seconds, and the network goes away and comes back.
#[tokio::test]
async fn an_rpc_call_made_while_the_connection_is_away_waits_for_it_and_is_served() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let network = Network::start(backend.address()).await;
    let alpha = backend.pair_through(&master, "alpha", &network.url).await;
    let mut work = on_device(&backend, &master, &vendor, &alpha).await;
    let home = alpha.runner.home_dir().to_owned();
    let script = "printf 'started\\n'; while [ ! -f go ]; do sleep 0.05; done; touch calling; demi shell status 1 > status.txt 2>&1; touch asked; while [ ! -f finish ]; do sleep 0.05; done";
    let command = started(&mut work, "t1", script).await;
    assert_eq!(command, "1");

    // The job asks the backend about itself while its runner is away: the
    // call waits, and the connection that comes back serves it
    // (`runner.md` § Command lifetime).
    network.cut();
    backend.until_online(&master, alpha.id(), false).await;
    std::fs::write(home.join("go"), "").unwrap();
    until_exists(&home.join("calling")).await;
    network.open();
    until_exists(&home.join("asked")).await;
    let status = std::fs::read_to_string(home.join("status.txt")).unwrap();
    assert!(status.starts_with("status: running"), "{status}");
    std::fs::write(home.join("finish"), "").unwrap();
    backend.close().await;
}

// About four seconds: a real device pairs through a network that goes away,
// and the command reports every second.
#[tokio::test]
async fn an_interval_report_while_the_hosts_connection_is_away_says_so_instead_of_counting_the_silence() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let network = Network::start(backend.address()).await;
    let alpha = backend.pair_through(&master, "alpha", &network.url).await;
    let mut work = on_device(&backend, &master, &vendor, &alpha).await;
    let home = alpha.runner.home_dir().to_owned();
    let script = "printf 'started\\n'; while [ ! -f finish ]; do sleep 0.05; done";
    let first = work.turn(vec![shell("t1", script, 1_000), say("waiting")]).await;
    let command = field(&first.received[0], "commandId").to_owned();
    work.script((0..20).map(|_| say("noted")).collect());

    network.cut();
    backend.until_online(&master, alpha.id(), false).await;
    let away = format!(
        "Command {command} (t1) is still running, as far as Demi knows: its Host has been unreachable for "
    );
    until_requested(&vendor, &away).await;
    let report = vendor
        .requests()
        .iter()
        .map(|request| request.json().to_string())
        .find(|request| request.contains(&away))
        .expect("a request carries the report");
    let report = &report[report.find(&away).unwrap()..];
    let report = &report[..report.find("\"").unwrap()];
    assert!(
        report.contains("and its runner keeps the command for up to 10m."),
        "{report}"
    );
    assert!(!report.contains("no output for"), "{report}");

    network.open();
    backend.until_online(&master, alpha.id(), true).await;
    std::fs::write(home.join("finish"), "").unwrap();
    until_requested(&vendor, &format!("Command {command} (t1) ended with exit code 0.")).await;
    backend.close().await;
}

/// A command that prints `seen`, which its call shows, then once `go` is
/// there prints `unseen` and waits.
const SEEN_THEN_UNSEEN: &str = "printf 'seen\\n'; while [ ! -f go ]; do sleep 0.05; done; printf 'unseen\\n'; sleep 60";

// About six seconds: a real device pairs, the calls that start the two
// commands each wait for two quiet seconds, and the device's runner starts
// anew while the model's next step, a look at one of them, is on its way.
#[tokio::test]
async fn a_loss_the_next_steps_look_shows_is_not_reported_and_another_reports_its_unseen_output() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let mut alpha = backend.pair(&master, "alpha").await;
    let mut work = on_device(&backend, &master, &vendor, &alpha).await;
    let home = alpha.runner.home_dir().to_owned();
    let looked_at = started(&mut work, "t1", SEEN_THEN_UNSEEN).await;
    let other = started(
        &mut work,
        "t2",
        "printf 'seen\\n'; while [ ! -f go ]; do sleep 0.05; done; printf 'other\\n'; sleep 60",
    )
    .await;
    std::fs::write(home.join("go"), "").unwrap();
    let mut printing = vec![(looked_at.clone(), "unseen"), (other.clone(), "other")];
    while !printing.is_empty() {
        if let ServerFrame::ShellOutput { status, .. } = work.socket.frame().await {
            let command = status.command();
            printing.retain(|(id, text)| !(command.command_id.as_str() == id && command.tail.contains(text)));
        }
    }

    // The step is held until the losses are known, so their reports arrive
    // while its response streams, before its look runs.
    let (open, held) = watch::channel(false);
    let look = format!("demi shell status {looked_at}");
    let before = work
        .start(vec![shell("t3", &look, 10_000).held(held), say("looked")])
        .await;
    vendor.received(before + 1).await;
    alpha.runner.kill().await;
    alpha.runner.start_again();
    // Both reports wait in the turn before the step runs: a report that
    // came while it ran would end its window.
    eventually("both losses' reports wait", || {
        let waiting = lost_reports_waiting(&harness);
        async move { waiting == 2 }
    })
    .await;
    open.send_replace(true);
    // The turn's running phase came before the ends.
    work.socket
        .until(|frame| matches!(frame, ServerFrame::Phase { phase: SessionPhase::Idle }))
        .await;

    // The look shows the loss and the output since the model's last look,
    // and that loss is not reported again (`runtime.md` § Command reports).
    let reason = demi_backend_remote_host::RUNNER_RESTARTED;
    let looked = &work.observe(before).received[0];
    assert_eq!(
        shown_output(looked),
        format!("status: lost: {reason}\ncommandId: {looked_at}\noutput:\nunseen\n"),
        "{looked}"
    );
    assert_eq!(requested(&vendor, &format!("Command {looked_at} (t1) was lost")), 0);
    // The other loss reports what the model had not seen, and the reason
    // only where it says why.
    let lost = format!("Command {other} (t2) was lost: {reason}. Start it again if it is still needed.");
    assert_ne!(requested(&vendor, &format!(r#"{lost}\noutput:\nother""#)), 0);
    backend.close().await;
}

// About six seconds: a real device pairs, the command reports every second,
// twice before the backend restarts and once after.
#[tokio::test]
async fn a_report_after_a_backend_restart_counts_from_the_commands_start_and_shows_only_unseen_output() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let mut work = on_device(&backend, &master, &vendor, &alpha).await;
    let home = alpha.runner.home_dir().to_owned();
    let script = "printf 'seen\\n'; while [ ! -f finish ]; do sleep 0.05; done";
    let first = work.turn(vec![shell("t1", script, 1_000), say("waiting")]).await;
    let ran = std::time::Instant::now();
    assert_eq!(shown_output(&first.received[0]), "seen\n", "{}", first.received[0]);
    let command = field(&first.received[0], "commandId").to_owned();
    work.script((0..20).map(|_| say("noted")).collect());
    let mut reports = Vec::new();
    while reports.len() < 2 {
        reports = wakeups(&backend, &master).await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }

    // The backend closes right after a report, well before the next one.
    let mut reports = reports.len();
    let last = reports;
    while reports == last {
        reports = wakeups(&backend, &master).await.len();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let address = backend.address();
    backend.close().await;
    let before = u64::try_from(ran.elapsed().as_millis()).unwrap();
    // The wall clock the backend reads has moved as long as the command ran.
    harness
        .clock
        .advance(jiff::SignedDuration::from_millis(i64::try_from(before).unwrap()));
    let backend = harness.start_at(address).await;
    let mut after = Vec::new();
    while after.len() <= reports {
        after = wakeups(&backend, &master).await;
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let report = &after[reports];
    let ReportEvent::Running { running_ms, .. } = report.event else {
        panic!("{report:?}");
    };
    assert!(running_ms >= before, "{running_ms} ms, though it ran {before} ms before the restart");
    assert_eq!(report.output, "", "the model saw the output before the restart");
    std::fs::write(home.join("finish"), "").unwrap();
    until_requested(&vendor, &format!("Command {command} (t1) ended with exit code 0.")).await;
    backend.close().await;
}

// About four seconds: a real device pairs, the call that starts the command
// waits for two quiet seconds, and the backend restarts.
#[tokio::test]
async fn a_look_after_a_backend_restart_counts_from_the_commands_start() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let mut work = on_device(&backend, &master, &vendor, &alpha).await;
    let home = alpha.runner.home_dir().to_owned();
    let command = started(&mut work, "t1", "printf 'started\\n'; while [ ! -f finish ]; do sleep 0.05; done").await;

    // The command has run a minute when the backend starts again.
    let address = backend.address();
    backend.close().await;
    harness.clock.advance(jiff::SignedDuration::from_secs(60));
    let backend = harness.start_at(address).await;
    backend.until_online(&master, alpha.id(), true).await;
    let provider = anthropic_at(&backend, &master, &vendor, "/lifetime").await;
    work.reconnect(&backend, &master, FIRST, &provider).await;
    let look = format!("demi shell status {command}");
    let looked = work.turn(vec![shell("t2", &look, 10_000), say("looked")]).await;
    let looked = &looked.received[0];
    let running_ms: u64 = field(shown_output(looked).as_str(), "runningMs").parse().unwrap();
    assert!(running_ms >= 60_000, "{looked}");
    std::fs::write(home.join("finish"), "").unwrap();
    backend.close().await;
}

// Several seconds: a real device pairs, the call waits for two quiet
// seconds, and the backend restarts.
//
// A command whose script ended before a backend restart, while its
// background task runs on, still names the task once the restarted backend
// takes it up: the runner's hello lists each job's outliving tasks. Before,
// the restarted backend had no list, and a look said only that the command
// keeps running.
#[tokio::test]
async fn a_backend_restart_keeps_the_background_tasks_that_outlive_a_commands_script() {
    let vendor = MockVendor::start().await;
    let harness = Harness::new();
    let (backend, master) = harness.start_set_up().await;
    let alpha = backend.pair(&master, "alpha").await;
    let mut work = on_device(&backend, &master, &vendor, &alpha).await;
    let home = alpha.runner.home_dir().to_owned();
    // The task looks at its own command once the backend is back.
    let script = "(while [ ! -f go ]; do sleep 0.05; done; demi shell status 1 > status.txt 2>&1; touch asked) & echo started";
    let command = started(&mut work, "t1", script).await;
    assert_eq!(command, "1");

    let address = backend.address();
    backend.close().await;
    let backend = harness.start_at(address).await;
    backend.until_online(&master, alpha.id(), true).await;
    std::fs::write(home.join("go"), "").unwrap();
    until_exists(&home.join("asked")).await;
    let status = std::fs::read_to_string(home.join("status.txt")).unwrap();
    assert!(status.starts_with("status: running"), "{status}");
    assert!(
        status.contains("\nthe script has ended; its background task \"( while") 
            && status.contains("keeps the command running, and stopping the command stops it\n"),
        "{status}"
    );
    backend.close().await;
}
