//! The `shell` tool's timing on a real runner (`runtime.md` § Dispatch and
//! failures, § The window, § Command reports): the calls of one response
//! start together, every one starts in the working directory whatever the
//! one before did, an agent message or a command report ends a window and a
//! human steer does not, Send Now ends one without stopping its command, a
//! command left running reports every interval and its end, a resident one
//! returns once its start-up output is quiet and reports only its end, and
//! a subagent stays live while a command it started runs. The `demi shell`
//! commands run in the backend, which these scenarios lack: the backend's
//! own scenarios cover them.
//!
//! A command that cannot end until another runs, one that waits for the
//! file the other makes, shows what runs together without measuring time:
//! run alone, it outlasts its window.

use std::{cell::RefCell, rc::Rc};

use demi_agent_server::testing::client_text;
use demi_agent_tools::testing::{field, shown_output};
use demi_conversation_socket_protocol::{ClientFrame, ServerFrame, ShellStatus};
use demi_provider_common::{
    InferenceItem, InferenceRequest, ProviderEvent, ResultPart, UserPart,
    testing::{ScriptedRuntime, Turn},
};
use demi_agent_server::ServerConfig;
use demi_conversation_socket_protocol::SubagentEvent;
use demi_host_interface::{RpcInvocation, testing::MemoryPort};
use demi_shared_types::{AgentMessage, AgentMessageEvent, BlockId, NodeId, Sender, Timestamp};
use tokio_util::sync::CancellationToken;
use serde_json::{Value, json};

use crate::support::{Fixture, conversation, exec, is_idle, reply, resident, tolerant, turn, within};

/// What the model was shown of each tool call it made, by the call's id, in
/// the order the request carries them.
type Results = Rc<RefCell<Vec<(String, String)>>>;

/// A model that makes `responses`' calls one response after another, each
/// response made from the results it has seen, and then ends the turn; it
/// records every result.
fn model(responses: Vec<Box<dyn Fn(&[(String, String)]) -> Vec<ProviderEvent>>>) -> (Vec<Turn>, Results) {
    let results: Results = Rc::default();
    let mut turns = Vec::new();
    let mut responses = responses.into_iter();
    let first = responses.next().expect("a response");
    turns.push(Turn::Events(first(&[])));
    for respond in responses.map(Some).chain([None]) {
        let results = results.clone();
        turns.push(Turn::Respond(Box::new(move |request| {
            record(&results, request);
            match &respond {
                Some(respond) => respond(&results.borrow()),
                None => reply("done"),
            }
        })));
    }
    (turns, results)
}

/// Adds the results `request` carries that `results` lacks.
fn record(results: &Results, request: &InferenceRequest) {
    let mut results = results.borrow_mut();
    for item in request.items.iter() {
        let InferenceItem::ToolResult {
            tool_use_id,
            output,
            ..
        } = item
        else {
            continue;
        };
        if results.iter().any(|(id, _)| id == tool_use_id.as_str()) {
            continue;
        }
        let text: Vec<&str> = output
            .iter()
            .filter_map(|part| match part {
                ResultPart::Text(text) => Some(text.as_str()),
                _ => None,
            })
            .collect();
        results.push((tool_use_id.clone(), text.join("\n")));
    }
}

/// The result of the call `id`.
fn result<'a>(results: &'a [(String, String)], id: &str) -> &'a str {
    results
        .iter()
        .find(|(call, _)| call == id)
        .map(|(_, text)| text.as_str())
        .unwrap_or_else(|| panic!("no result of {id}: {results:#?}"))
}


/// A script that waits until the file `flag` exists, then prints
/// `through`.
const AWAIT_FLAG: &str = "until [ -e flag ]; do sleep 0.05; done; echo through";

/// A script that makes the file the other waits for.
const RAISE_FLAG: &str = "touch flag";

// About two seconds: two scripts, each a shell job.
#[tokio::test(flavor = "local")]
async fn a_responses_commands_run_together_and_their_results_keep_the_calls_order() {
    within(async {
        // The reader can end only once the writer runs: called one after
        // the other, the reader would outlast its window.
        let (turns, results) = model(vec![Box::new(|_| {
            vec![
                exec("reader", AWAIT_FLAG, 20_000),
                exec("writer", RAISE_FLAG, 20_000),
            ]
        })]);
        let script = ScriptedRuntime::new(turns);
        let fixture = Fixture::start(&script).await;
        let mut client = fixture.opened().await;
        turn(&mut client, "message-1", "Pass the flag.").await;

        let results = results.borrow();
        let order: Vec<&str> = results.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(order, ["reader", "writer"]);
        let reader = result(&results, "reader");
        assert_eq!(field(reader, "status"), "exited", "{reader}");
        assert_eq!(shown_output(reader), "through\n");
        assert_eq!(field(result(&results, "writer"), "status"), "exited");
        fixture.stop().await;
    })
    .await;
}

// About two seconds: four scripts, each a shell job.
#[tokio::test(flavor = "local")]
async fn every_exec_starts_in_the_working_directory_whatever_the_one_before_did() {
    within(async {
        let (turns, results) = model(vec![
            Box::new(|_| vec![exec("enter", "mkdir -p sub/one sub/two && cd sub", 20_000)]),
            Box::new(|_| {
                vec![
                    exec("first", "pwd && cd sub/one", 20_000),
                    exec("second", "pwd && cd sub/two", 20_000),
                ]
            }),
            Box::new(|_| vec![exec("after", "pwd", 20_000)]),
        ]);
        let script = ScriptedRuntime::new(turns);
        let fixture = Fixture::start(&script).await;
        let mut client = fixture.opened().await;
        turn(&mut client, "message-1", "Look around.").await;

        let results = results.borrow();
        let workspace = format!("{}\n", fixture.workspace);
        for id in ["first", "second", "after"] {
            assert_eq!(shown_output(result(&results, id)), workspace, "{id}");
        }
        fixture.stop().await;
    })
    .await;
}

/// The text of the first steer `request` carries.
fn steer_text(request: &InferenceRequest) -> Option<String> {
    request.items.iter().find_map(|item| match item {
        InferenceItem::UserSteer { content } => content.iter().find_map(|part| match part {
            UserPart::Text(text) => Some(text.clone()),
            _ => None,
        }),
        _ => None,
    })
}

/// Whether `frame` shows the command the call `call` started.
fn shows_call(frame: &ServerFrame, call: &str) -> bool {
    matches!(frame, ServerFrame::ShellOutput { status, .. } if status.command().tool_use_id == call)
}

/// A message to the conversation's root from a subagent.
fn agent_message(content: &str) -> AgentMessage {
    AgentMessage {
        id: BlockId::try_from("subagent:child:1").unwrap(),
        sender: Some(Sender {
            id: NodeId::try_from("child").unwrap(),
            number: 1,
            description: "Test triage".into(),
            round: 1,
        }),
        recipient_id: conversation(),
        timestamp: Timestamp::UNIX_EPOCH,
        content: content.to_owned(),
        event: AgentMessageEvent::Message {},
    }
}

// About a second: two scripts, each a shell job.
#[tokio::test(flavor = "local")]
async fn a_human_steer_waits_for_the_window_and_an_agent_message_ends_it() {
    within(async {
        let steered: Rc<RefCell<Option<String>>> = Rc::default();
        let seen = steered.clone();
        let results: Results = Rc::default();
        let recorded = results.clone();
        let turns = vec![
            // Windows far longer than the test.
            Turn::Events(vec![exec("flagged", AWAIT_FLAG, 600_000)]),
            Turn::Respond(Box::new(move |request| {
                record(&recorded, request);
                *seen.borrow_mut() = steer_text(request);
                vec![exec("second", "until [ -e second ]; do sleep 0.05; done", 600_000)]
            })),
            Turn::Respond(Box::new({
                let recorded = results.clone();
                move |request| {
                    record(&recorded, request);
                    reply("done")
                }
            })),
        ];
        let script = ScriptedRuntime::new(turns);
        let fixture = Fixture::start(&script).await;
        let mut client = fixture.opened().await;
        client
            .send(ClientFrame::Send {
                message_id: "message-1".try_into().unwrap(),
                content: client_text("Wait for the flag."),
            })
            .await;
        // The command runs: the user steers, and the steer waits for the
        // call, which returns only once the command ends.
        client.next_until(|frame| shows_call(frame, "flagged")).await;
        client
            .send(ClientFrame::Steer {
                steer_id: "steer-1".try_into().unwrap(),
                content: client_text("Then look at the log."),
            })
            .await;
        client
            .next_until(|frame| matches!(frame, ServerFrame::SteerResult { .. }))
            .await;
        std::fs::write(format!("{}/flag", fixture.workspace), "").unwrap();
        // The next command runs: a subagent's message ends its window.
        client.next_until(|frame| shows_call(frame, "second")).await;
        let root = fixture.server.node(&conversation(), &conversation()).unwrap();
        root.session()
            .accept_agent_message(agent_message("The failing test is flaky."))
            .await
            .unwrap();
        client.next_until(is_idle).await;

        let results = results.borrow();
        let flagged = result(&results, "flagged");
        assert_eq!(field(flagged, "status"), "exited", "{flagged}");
        assert_eq!(shown_output(flagged), "through\n");
        assert_eq!(steered.borrow().as_deref(), Some("Then look at the log."));
        let second = result(&results, "second");
        assert_eq!(field(second, "status"), "running", "{second}");
        assert!(!second.contains("moved to the background"), "{second}");
        fixture.stop().await;
    })
    .await;
}

/// Replies that end a turn, as many as the reports of a scenario's commands
/// take: how many continuations a command's reports open depends on when
/// they arrive.
fn replies(count: usize) -> impl Iterator<Item = Turn> {
    (0..count).map(|_| Turn::Events(reply("noted")))
}

/// The texts of the user messages and steers `request` carries.
fn inputs(request: &InferenceRequest) -> Vec<String> {
    request
        .items
        .iter()
        .filter_map(|item| match item {
            InferenceItem::UserMessage { content } | InferenceItem::UserSteer { content } => {
                Some(content)
            }
            _ => None,
        })
        .flatten()
        .filter_map(|part| match part {
            UserPart::Text(text) => Some(text.clone()),
            _ => None,
        })
        .collect()
}

/// Waits until a request of `script` carries an input that holds `text`,
/// and answers that input.
async fn reported(script: &ScriptedRuntime, text: &str) -> String {
    loop {
        let found = script
            .requests()
            .iter()
            .flat_map(inputs)
            .find(|input| input.contains(text));
        if let Some(found) = found {
            return found;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

/// The tool results of every request of `script`, by call id, with whether
/// each is an error to the provider.
fn errors(script: &ScriptedRuntime) -> Vec<(String, bool)> {
    let mut seen: Vec<(String, bool)> = Vec::new();
    for request in script.requests() {
        for item in request.items.iter() {
            if let InferenceItem::ToolResult {
                tool_use_id,
                is_error,
                ..
            } = item
                && !seen.iter().any(|(id, _)| id == tool_use_id)
            {
                seen.push((tool_use_id.clone(), *is_error));
            }
        }
    }
    seen
}

// About a second: two scripts, each a shell job.
#[tokio::test(flavor = "local")]
async fn send_now_on_a_steer_moves_the_command_to_the_background_and_the_turn_goes_on() {
    within(async {
        let turns = [
            // A window far longer than the test.
            Turn::Events(vec![exec(
                "wait",
                "until [ -e go ]; do sleep 0.05; done; echo went",
                600_000,
            )]),
            Turn::Events(vec![exec("go", "touch go", 20_000)]),
        ]
        .into_iter()
        .chain(replies(3));
        let script = ScriptedRuntime::new(turns);
        let fixture = Fixture::start(&script).await;
        let mut client = fixture.opened().await;
        client
            .send(ClientFrame::Send {
                message_id: "message-1".try_into().unwrap(),
                content: client_text("Wait for go."),
            })
            .await;
        // The command runs: the user steers and sends the steer now.
        client.next_until(|frame| shows_call(frame, "wait")).await;
        client
            .send(ClientFrame::Steer {
                steer_id: "steer-1".try_into().unwrap(),
                content: client_text("Say go."),
            })
            .await;
        client
            .next_until(|frame| matches!(frame, ServerFrame::SteerResult { .. }))
            .await;
        client
            .send(ClientFrame::SteerNow {
                steer_id: "steer-1".try_into().unwrap(),
            })
            .await;

        // The command ran on: it ended once the next call let it, and its end
        // was reported.
        let command = fixture_command(&script, "wait").await;
        let report = reported(
            &script,
            &format!("Command {command} (Run the test script) ended with exit code 0"),
        )
        .await;
        // It carries what the command printed since the result's look.
        assert_eq!(
            report,
            format!("Command {command} (Run the test script) ended with exit code 0.\noutput:\nwent")
        );
        let requests = script.requests();
        let results: Results = Rc::default();
        for request in &requests {
            record(&results, request);
        }
        let results = results.borrow();
        let waited = result(&results, "wait");
        assert_eq!(field(waited, "status"), "running", "{waited}");
        let lines: Vec<&str> = waited.lines().collect();
        let line = format!(
            "[The user sent a message, so command {command} moved to the background. It keeps running.]"
        );
        let at = lines.iter().position(|shown| *shown == line).expect(waited);
        assert!(lines[at + 1].starts_with("next:"), "{waited}");
        assert_eq!(steer_text(&requests[1]).as_deref(), Some("Say go."));
        assert!(!fixture.kinds().contains(&"abort".to_owned()));
        fixture.stop().await;
    })
    .await;
}

/// A server whose commands report at intervals from a tenth of a second,
/// so that a scenario sees them within its time.
fn reporting() -> ServerConfig {
    ServerConfig {
        interval_floor_ms: 100,
        ..ServerConfig::default()
    }
}

// About two seconds: two scripts, each a shell job, and the reports of one
// of them every half second.
#[tokio::test(flavor = "local")]
async fn a_command_left_running_reports_every_interval_and_its_end_and_a_failed_one_is_an_error() {
    within(async {
        let turns = [Turn::Events(vec![
            exec("fail", "echo broken >&2; exit 2", 20_000),
            exec(
                "suite",
                "echo started; until [ -e done ]; do sleep 0.05; done; echo finished; exit 3",
                500,
            ),
        ])]
        .into_iter()
        .chain(replies(40));
        let script = ScriptedRuntime::new(turns);
        let fixture = Fixture::start_with(&script, reporting()).await;
        let mut client = fixture.opened().await;
        turn(&mut client, "message-1", "Run the suite.").await;

        let suite = fixture.shell_commands()[1].clone();
        // The command still runs: it reports what a look shows of it.
        let progress = reported(
            &script,
            &format!("Command {suite} (Run the test script) is still running after"),
        )
        .await;
        // The result showed `started`, so the report shows nothing new.
        assert!(progress.contains(".\noutput: (empty)\n"), "{progress}");
        assert!(
            progress.ends_with(&format!(
                "It reports every 1s; change that with demi shell status {suite} --interval <duration>, or with --resident to hear only of its end."
            )),
            "{progress}"
        );
        std::fs::write(format!("{}/done", fixture.workspace), "").unwrap();
        // Its end carries the output since the last look, as a result
        // shows a command's end, so the model needs no look.
        let end = reported(
            &script,
            &format!("Command {suite} (Run the test script) ended with exit code 3"),
        )
        .await;
        assert_eq!(
            end,
            format!("Command {suite} (Run the test script) ended with exit code 3.\noutput:\nfinished")
        );

        // The failed command's result is an error to the provider; the one
        // that still ran is not.
        let errors = errors(&script);
        assert!(errors.contains(&("fail".to_owned(), true)), "{errors:?}");
        assert!(errors.contains(&("suite".to_owned(), false)), "{errors:?}");
        // A look moves the model's place: the output the first report showed
        // is not shown again.
        let started = script
            .requests()
            .iter()
            .flat_map(inputs)
            .filter(|input| input.contains("\nstarted\n") || input.ends_with("\nstarted"))
            .count();
        assert!(started <= 1, "{started}");
        fixture.stop().await;
    })
    .await;
}


// About three seconds: two scripts, each a shell job, and the two seconds of
// quiet that end a resident command's call.
#[tokio::test(flavor = "local")]
async fn a_resident_command_returns_once_quiet_reports_only_its_end_and_its_end_ends_a_window() {
    within(async {
        let turns = [
            Turn::Events(vec![resident(
                "serve",
                "echo ready; until [ -e stop ]; do sleep 0.05; done",
            )]),
            // A window far longer than the test, which the server's end
            // ends.
            Turn::Events(vec![exec(
                "long",
                "touch stop; until [ -e never ]; do sleep 0.05; done",
                600_000,
            )]),
        ]
        .into_iter()
        .chain(replies(3));
        let script = ScriptedRuntime::new(turns);
        let fixture = Fixture::start_with(&script, reporting()).await;
        let mut client = fixture.opened().await;
        let started = tokio::time::Instant::now();
        client
            .send(ClientFrame::Send {
                message_id: "message-1".try_into().unwrap(),
                content: client_text("Serve, then stop."),
            })
            .await;
        let serve = fixture_command(&script, "serve").await;
        let served = started.elapsed();

        let report = reported(
            &script,
            &format!("Command {serve} (Run the test script) ended with exit code 0"),
        )
        .await;
        client.next_until(is_idle).await;

        // The call returned once the start-up output had been quiet for two
        // seconds, long before its thirty.
        assert!(
            served >= std::time::Duration::from_secs(2) && served < std::time::Duration::from_secs(30),
            "{served:?}"
        );
        let results: Results = Rc::default();
        for request in &script.requests() {
            record(&results, request);
        }
        let results = results.borrow();
        let first = result(&results, "serve");
        assert_eq!(field(first, "status"), "running", "{first}");
        assert_eq!(shown_output(first), "ready\n");
        assert!(first.contains("reports to you when it ends"), "{first}");
        // The report ended the long call's window: its command runs on.
        let long = result(&results, "long");
        assert_eq!(field(long, "status"), "running", "{long}");
        // It printed nothing after the result showed `ready`.
        assert_eq!(
            report,
            format!("Command {serve} (Run the test script) ended with exit code 0.\noutput: (empty)")
        );
        // It reported nothing while it ran.
        assert!(
            !script
                .requests()
                .iter()
                .flat_map(inputs)
                .any(|input| input.contains("is still running")),
        );
        fixture.stop().await;
    })
    .await;
}

/// The command the call `id` started, once its result reached the model.
async fn fixture_command(script: &ScriptedRuntime, id: &str) -> String {
    loop {
        let results: Results = Rc::default();
        for request in &script.requests() {
            record(&results, request);
        }
        if let Some((_, text)) = results.borrow().iter().find(|(call, _)| call == id) {
            return field(text, "commandId").to_owned();
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
}

// About three seconds: a script a subagent runs as a shell job, and the two
// seconds of quiet that end its call.
#[tokio::test(flavor = "local")]
async fn a_subagent_whose_command_runs_stays_live_and_closes_once_it_ended_and_the_child_answered() {
    within(async {
        let script = ScriptedRuntime::new([
            // The child's runs.
            Turn::Events(vec![resident(
                "serve",
                "echo ready; until [ -e stop ]; do sleep 0.05; done",
            )]),
            Turn::Events(reply("The server runs.")),
            Turn::Events(reply("The server stopped.")),
            // The root's, which its child's completion opens.
            Turn::Events(reply("noted")),
        ]);
        let fixture = Fixture::start(&script).await;
        let mut client = fixture.opened().await;
        spawn(&fixture, "Serve until stopped.").await;
        let started = client
            .next_until(|frame| {
                matches!(frame, ServerFrame::Subagent { event: SubagentEvent::Started, .. })
            })
            .await;
        let Some(ServerFrame::Subagent { job, .. }) = started.last() else {
            unreachable!()
        };
        let child = job.subagent_id.clone();
        let tree = fixture.server.tree(&conversation()).unwrap();

        // The child answered and its command runs: it stays live.
        while script.requests().len() < 2 {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        let node = fixture.server.node(&conversation(), &child).unwrap();
        node.session().settled().await;
        assert!(node.session().status().commands);
        assert!(fixture.server.node(&conversation(), &child).is_some());
        assert!(!tree.is_quiescent());

        std::fs::write(format!("{}/stop", fixture.workspace), "").unwrap();
        client
            .next_until(|frame| {
                matches!(frame, ServerFrame::Subagent { event: SubagentEvent::Closed, job }
                    if job.subagent_id == child && job.result.as_deref() == Some("The server stopped."))
            })
            .await;
        client.next_until(is_idle).await;
        assert_eq!(script.remaining(), 0);
        fixture.stop().await;
    })
    .await;
}

/// Spawns a child of the root with `prompt` as the root's job would, through
/// the root's `demi agent spawn`.
async fn spawn(fixture: &Fixture, prompt: &str) {
    let (code, _, stderr) = root_call(fixture, &["agent", "spawn"], json!({ "prompt": prompt })).await;
    assert_eq!(code, 0, "{stderr}");
}

/// Runs `demi <path>` with `args` as a job of the root would, and answers
/// its exit code, stdout and stderr.
async fn root_call(fixture: &Fixture, path: &[&str], args: Value) -> (u8, String, String) {
    let root = fixture.server.node(&conversation(), &conversation()).unwrap();
    let path: Vec<&str> = std::iter::once("demi").chain(path.iter().copied()).collect();
    let mut invocation: RpcInvocation = serde_json::from_value(json!({
        "path": path,
        "argv": [],
        "args": args,
        "json": false,
        "host": "device",
        "cwd": fixture.workspace,
        "env": {},
        "context": {
            "conversation": conversation(),
            "caller": { "kind": "agent", "number": 0 },
            "locale": { "timeZone": "UTC", "languages": ["en"] },
            "colorScheme": "light",
        },
        "stdin": false,
    }))
    .expect("the invocation is well formed");
    invocation.caller = Some(root.job_caller());
    let port = MemoryPort::new();
    let code = root
        .commands()
        .dispatch(invocation, port.port(CancellationToken::new()))
        .await
        .expect("the call is dispatched");
    let text = |bytes: Vec<u8>| String::from_utf8(bytes).expect("the output is text");
    (code, text(port.stdout()), text(port.stderr()))
}

// About five seconds: a subagent's command runs as a shell job and reports
// to it three seconds after its call returned, after the parent looked.
#[tokio::test(flavor = "local")]
async fn a_look_at_another_agents_command_shows_the_output_since_the_lookers_own_last_look() {
    within(async {
        let script = tolerant(vec![
            // The child's runs.
            vec![exec(
                "serve",
                "echo one; until [ -e next ]; do sleep 0.05; done; echo two; until [ -e stop ]; do sleep 0.05; done",
                3_000,
            )],
            reply("The server runs."),
        ]);
        let fixture = Fixture::start_with(&script, reporting()).await;
        let _client = fixture.opened().await;
        spawn(&fixture, "Serve.").await;
        let serve = fixture_command(&script, "serve").await;
        let look = || async {
            let (code, stdout, stderr) =
                root_call(&fixture, &["shell", "status"], json!({ "id": [serve.clone()] })).await;
            assert_eq!(code, 0, "{stderr}");
            stdout
        };

        // The parent's first look shows all the output so far, though the
        // child saw it in its call's result.
        let first = look().await;
        assert!(first.contains("\noutput:\none\n"), "{first}");
        std::fs::write(format!("{}/next", fixture.workspace), "").unwrap();
        // Its next looks show only what came since: never `one` again.
        let mut later = look().await;
        while !later.contains("\ntwo\n") {
            assert!(!later.contains("one"), "{later}");
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            later = look().await;
        }
        assert!(later.contains("\noutput:\ntwo\n") && !later.contains("one"), "{later}");
        // The parent's looks moved only its own place: the child's report
        // still shows it `two`.
        let report = reported(&script, &format!("Command {serve} (Run the test script) is still running")).await;
        assert!(report.contains("\noutput:\ntwo"), "{report}");
        fixture.stop().await;
    })
    .await;
}

// About three seconds: two scripts, each a shell job, and the two seconds
// of quiet that end the first call.
#[tokio::test(flavor = "local")]
async fn a_stop_ends_the_command_a_call_watches_and_leaves_one_an_earlier_call_left_running() {
    within(async {
        let script = tolerant(vec![
            vec![resident("serve", "echo ready; until [ -e stop ]; do sleep 0.05; done")],
            // A window far longer than the test, which the Stop ends.
            vec![exec("slow", "until [ -e never ]; do sleep 0.05; done", 600_000)],
        ]);
        let fixture = Fixture::start(&script).await;
        let mut client = fixture.opened().await;
        client
            .send(ClientFrame::Send {
                message_id: "message-1".try_into().unwrap(),
                content: client_text("Serve, then wait."),
            })
            .await;
        client.next_until(|frame| shows_call(frame, "slow")).await;
        client.send(ClientFrame::Abort {}).await;
        client
            .next_until(|frame| matches!(frame, ServerFrame::AbortResult { .. }))
            .await;
        // The watched command ended with its call.
        client
            .next_until(|frame| {
                matches!(frame, ServerFrame::ShellOutput { status, .. }
                    if status.command().tool_use_id == "slow"
                        && !matches!(**status, ShellStatus::Running { .. }))
            })
            .await;

        // The server an earlier call left running runs on, and its end
        // wakes the stopped node.
        let serve = fixture_command(&script, "serve").await;
        std::fs::write(format!("{}/stop", fixture.workspace), "").unwrap();
        reported(
            &script,
            &format!("Command {serve} (Run the test script) ended with exit code 0."),
        )
        .await;
        fixture.stop().await;
    })
    .await;
}
