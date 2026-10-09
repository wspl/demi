//! The shell tools' timing on a real runner (`runtime.md` § Dispatch and
//! failures, § The window, § Stopping a command): the `shell_exec` calls of
//! one response start together and a call of another tool splits them, the
//! first takes the default shell and the others start beside it, an agent
//! message ends a window and a human steer does not, Send Now ends one
//! without stopping its command, and `shell_status` watches a command up to
//! its end. `demi shell stop` runs in the backend, which these
//! scenarios lack: the backend's own scenarios cover it.
//!
//! A command that cannot end until another runs, one that waits for the
//! file the other makes, shows what runs together without measuring time:
//! run alone, it outlasts its window.

use std::{cell::RefCell, rc::Rc};

use demi_agent_server::testing::client_text;
use demi_agent_tools::testing::{field, shown_output};
use demi_conversation_socket_protocol::{ClientFrame, ServerFrame};
use demi_provider_common::{
    InferenceItem, InferenceRequest, ProviderEvent, ResultPart, UserPart,
    testing::{ScriptedRuntime, Turn, event},
};
use demi_shared_types::{AgentMessage, AgentMessageEvent, BlockId, NodeId, Sender, Timestamp};
use serde_json::{Value, json};

use crate::support::{Fixture, conversation, exec, is_idle, reply, turn, within};

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

fn call(id: &str, tool: &str, input: Value) -> ProviderEvent {
    event::tool_call(id, tool, input)
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

// About three seconds: four scripts, each a shell job.
#[tokio::test(flavor = "local")]
async fn a_call_between_two_execs_splits_them() {
    within(async {
        let (turns, results) = model(vec![
            Box::new(|_| vec![exec("held", "read line", 200)]),
            // The look at the held command is a step of its own, so the
            // reader runs alone, and its window passes.
            Box::new(|results| {
                let held = field(result(results, "held"), "commandId").to_owned();
                vec![
                    exec("reader", AWAIT_FLAG, 500),
                    call("look", "shell_status", json!({"commandId": held})),
                    exec("writer", RAISE_FLAG, 20_000),
                ]
            }),
            Box::new(|results| {
                let reader = field(result(results, "reader"), "commandId").to_owned();
                vec![call(
                    "wait",
                    "shell_status",
                    json!({"commandId": reader, "timeoutMs": 600_000}),
                )]
            }),
        ]);
        let script = ScriptedRuntime::new(turns);
        let fixture = Fixture::start(&script).await;
        let mut client = fixture.opened().await;
        turn(&mut client, "message-1", "Pass the flag.").await;

        let results = results.borrow();
        for running in ["reader", "look"] {
            let text = result(&results, running);
            assert_eq!(field(text, "status"), "running", "{text}");
        }
        assert_eq!(field(result(&results, "writer"), "status"), "exited");
        // The reader ended once the writer ran.
        let waited = result(&results, "wait");
        assert_eq!(field(waited, "status"), "exited", "{waited}");
        assert_eq!(shown_output(waited), "through\n");
        fixture.stop().await;
    })
    .await;
}

// About two seconds: four scripts, each a shell job.
#[tokio::test(flavor = "local")]
async fn the_first_exec_keeps_its_directory_and_the_others_start_beside_it() {
    within(async {
        let (turns, results) = model(vec![
            Box::new(|_| vec![exec("enter", "mkdir -p sub/one sub/two && cd sub", 20_000)]),
            Box::new(|_| {
                vec![
                    exec("first", "pwd && cd one", 20_000),
                    exec("second", "pwd && cd two", 20_000),
                ]
            }),
            Box::new(|_| vec![exec("after", "pwd", 20_000)]),
        ]);
        let script = ScriptedRuntime::new(turns);
        let fixture = Fixture::start(&script).await;
        let mut client = fixture.opened().await;
        turn(&mut client, "message-1", "Look around.").await;

        let results = results.borrow();
        let sub = format!("{}/sub", fixture.workspace);
        // The second runs in a new shell that starts where the default
        // shell was when the step started.
        assert_eq!(shown_output(result(&results, "first")), format!("{sub}\n"));
        assert_eq!(shown_output(result(&results, "second")), format!("{sub}\n"));
        // Only the default shell's directory carries over.
        assert_eq!(shown_output(result(&results, "after")), format!("{sub}/one\n"));
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

// About a second: two scripts, each a shell job.
#[tokio::test(flavor = "local")]
async fn send_now_on_a_steer_moves_the_command_to_the_background_and_the_turn_goes_on() {
    within(async {
        let steered: Rc<RefCell<Option<String>>> = Rc::default();
        let seen = steered.clone();
        let results: Results = Rc::default();
        let recorded = results.clone();
        let turns = vec![
            // A window far longer than the test.
            Turn::Events(vec![exec("read", "read line; echo \"got $line\"", 600_000)]),
            Turn::Respond(Box::new(move |request| {
                record(&recorded, request);
                *seen.borrow_mut() = steer_text(request);
                let command = field(result(&recorded.borrow(), "read"), "commandId").to_owned();
                vec![call(
                    "answer",
                    "shell_status",
                    json!({"commandId": command, "stdin": "go\n", "description": "Answer the prompt", "timeoutMs": 600_000}),
                )]
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
                content: client_text("Read a line."),
            })
            .await;
        // The command runs: the user steers and sends the steer now.
        client.next_until(|frame| shows_call(frame, "read")).await;
        client
            .send(ClientFrame::Steer {
                steer_id: "steer-1".try_into().unwrap(),
                content: client_text("Answer go."),
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
        client.next_until(is_idle).await;

        let results = results.borrow();
        let read = result(&results, "read");
        assert_eq!(field(read, "status"), "running", "{read}");
        let command = field(read, "commandId");
        let lines: Vec<&str> = read.lines().collect();
        let line = format!(
            "[The user sent a message, so command {command} moved to the background. It keeps running.]"
        );
        let at = lines.iter().position(|shown| *shown == line).expect(read);
        assert!(lines[at + 1].starts_with("next:"), "{read}");
        assert_eq!(steered.borrow().as_deref(), Some("Answer go."));
        // The command ran on: it read the answer and ended.
        let answered = result(&results, "answer");
        assert_eq!(field(answered, "status"), "exited", "{answered}");
        assert_eq!(shown_output(answered), "got go\n");
        assert!(!fixture.kinds().contains(&"abort".to_owned()));
        fixture.stop().await;
    })
    .await;
}

// About two seconds: four scripts, each a shell job.
#[tokio::test(flavor = "local")]
async fn a_shell_a_call_names_is_never_given_to_a_call_that_names_none() {
    within(async {
        let (turns, results) = model(vec![
            Box::new(|_| vec![exec("enter", "mkdir -p sub && cd sub && sleep 0.3", 50)]),
            Box::new(|results| {
                let enter = field(result(results, "enter"), "commandId").to_owned();
                vec![call(
                    "ended",
                    "shell_status",
                    json!({"commandId": enter, "timeoutMs": 20_000}),
                )]
            }),
            // The call without a shell comes first: given the default shell,
            // it would leave the call that names that shell refused as busy.
            Box::new(|results| {
                let default = field(result(results, "enter"), "shellId").to_owned();
                vec![
                    exec("unnamed", "pwd", 20_000),
                    call(
                        "named",
                        "shell_exec",
                        json!({"description": "Run the test script", "script": "pwd", "timeoutMs": 20_000, "shellId": default}),
                    ),
                ]
            }),
        ]);
        let script = ScriptedRuntime::new(turns);
        let fixture = Fixture::start(&script).await;
        let mut client = fixture.opened().await;
        turn(&mut client, "message-1", "Look around.").await;

        let results = results.borrow();
        let sub = format!("{}/sub\n", fixture.workspace);
        for id in ["unnamed", "named"] {
            let text = result(&results, id);
            assert_eq!(field(text, "status"), "exited", "{text}");
            assert_eq!(shown_output(text), sub);
        }
        fixture.stop().await;
    })
    .await;
}
