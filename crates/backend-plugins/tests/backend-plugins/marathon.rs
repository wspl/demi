//! The coding agent at work: the model edits files with `demi file` and
//! controls running commands with the shell tools, on a real runner.

use std::{cell::RefCell, rc::Rc};

use demi_agent_tools::testing::{field, shown_output};
use demi_conversation_socket_protocol::{ClientFrame, ServerFrame, ShellStatus};
use demi_provider_common::{
    InferenceItem, InferenceRequest, ProviderEvent, UserPart,
    testing::{ScriptedRuntime, Turn, event},
};
use demi_shared_types::CommandId;
use serde_json::json;

use crate::support::{Fixture, exec, is_idle, last_result, reply, scripts, turn, within};

// Several seconds: five scripts over two messages run a shell job each, and
// the first `demi file` starts the `demi.file` service.
#[tokio::test(flavor = "local")]
async fn a_coding_workflow_edits_files_and_keeps_its_shell_across_messages() {
    within(async {
        let (turns, recorded) = scripts(&[
            &[
                "demi file create src/app.ts <<'EOF'\nexport const value = 1\nEOF",
                "grep -q 'value = 2' src/app.ts",
                "demi file edit src/app.ts --old \"1\" --new \"2\" && cd src",
                "grep -q 'value = 2' app.ts && echo passed",
            ],
            &["pwd"],
        ]);
        let script = ScriptedRuntime::new(turns);
        let fixture = Fixture::start(&script).await;
        let mut client = fixture.opened().await;

        turn(
            &mut client,
            "message-1",
            "Create the app file, then fix the value.",
        )
        .await;
        let results = recorded.borrow().clone();
        assert_eq!(results.len(), 4, "{results:#?}");
        for result in &results {
            assert!(result.starts_with("status: exited\n"), "{result}");
        }
        assert_eq!(field(&results[0], "exitCode"), "0");
        assert_eq!(shown_output(&results[0]), "Created src/app.ts\n");
        // The failing check, then the fix and the passing one.
        assert_eq!(field(&results[1], "exitCode"), "1");
        assert_eq!(shown_output(&results[2]), "Edited src/app.ts\n");
        assert_eq!(shown_output(&results[3]), "passed\n");
        assert_eq!(
            fixture.kinds(),
            [
                "user",
                "tool_call:completed",
                "tool_call:completed",
                "tool_call:completed",
                "tool_call:completed",
                "text",
                "response"
            ]
        );

        // A command whose result showed everything was let go: the client
        // cannot reach it either.
        let released = fixture.shell_commands()[0].clone();
        client
            .send(ClientFrame::ShellAbort {
                command_id: released,
            })
            .await;
        let answer = client.received();
        assert!(
            matches!(&answer[..], [ServerFrame::Error { .. }]),
            "{answer:?}"
        );

        // The next message finds the directory the last script ended in.
        turn(&mut client, "message-2", "Where are you?").await;
        let last = recorded.borrow()[4].clone();
        assert_eq!(shown_output(&last), format!("{}/src\n", fixture.workspace));
        assert_eq!(
            std::fs::read_to_string(format!("{}/src/app.ts", fixture.workspace)).unwrap(),
            "export const value = 2\n"
        );

        // The model was given the three tools.
        let first = &script.requests()[0];
        let tools: Vec<&str> = first.tools.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(tools, ["shell_exec", "shell_status", "yield"]);
        // The prompt opens with the product's identity and harness guide,
        // indexes the node's groups, the plugins' `demi file` and the
        // `demi agent` graft, without their manuals, and names the model
        // last.
        let prompt = &first.system_prompt;
        assert!(
            prompt.starts_with("You are a coding agent.\n\nHow Demi works.\n\n"),
            "{prompt}"
        );
        for taught in [
            "Capabilities:",
            "demi agent\nRuns helper agents",
            "Operations: spawn, send, abort, resume, list, show, profiles\n",
            "demi file\nReads, creates, edits and patches files precisely",
            "Operations: read, create, edit, patch\n",
            "Details: demi file --help; one operation: demi file <operation> --help",
        ] {
            assert!(prompt.contains(taught), "{taught}");
        }
        for absent in ["Usage:", "Stdin body:", "demi file create <path>", "--content"] {
            assert!(!prompt.contains(absent), "{absent}");
        }
        assert!(
            prompt.ends_with(
                "This conversation runs on test-model (stub, test-model). If asked which model you are, answer with this."
            ),
            "{prompt}"
        );
        fixture.stop().await;
    })
    .await;
}

/// Where the model is in the flow below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// The reader runs; the model checks it once.
    Started,
    /// The reader waits for its name; the model writes it.
    Checked,
    /// The reader waits for its second line, which the user types; the
    /// model waits for the reader's end.
    Fed,
    /// The model yielded until the reader ends.
    Yielded,
    /// The reader's end woke the model, which read its end.
    Read,
    /// The long command runs; the model waits for its end.
    Long,
    /// The model yielded until the long command ends, which the user stops.
    LongYielded,
    /// The stop woke the model, which looks at the long command.
    Stopped,
    /// The model saw the long command stopped and ended the turn.
    Seen,
}

/// The model's side of the flow: it answers each request from the result
/// it carries, and records what it saw for the test to check.
struct Model {
    phase: Phase,
    reader: Option<CommandId>,
    long: Option<CommandId>,
    /// The reader's output, from every result that showed some.
    reader_output: String,
    /// The text of the wakeup the reader's end fired.
    wakeup: Option<String>,
    /// The text of the wakeup the long command's stop fired.
    stop_wakeup: Option<String>,
    polls: u32,
    /// Every result, in order.
    seen: Vec<String>,
}

impl Model {
    fn answer(&mut self, request: &InferenceRequest) -> Vec<ProviderEvent> {
        let result = last_result(request);
        self.seen.push(result.clone());
        match self.phase {
            Phase::Started => {
                self.reader = Some(command_id(&result));
                self.phase = Phase::Checked;
                vec![self.call("shell_status", json!({"commandId": self.reader}))]
            }
            Phase::Checked => {
                self.phase = Phase::Fed;
                vec![self.call(
                    "shell_status",
                    json!({"commandId": self.reader, "stdin": "Alice\n", "description": "Answer with the name"}),
                )]
            }
            Phase::Fed => {
                self.reader_output.push_str(&shown_output(&result));
                assert_eq!(field(&result, "status"), "running", "{result}");
                self.phase = Phase::Yielded;
                // Far longer than the test: only the reader's end wakes it.
                vec![self.call(
                    "yield",
                    json!({"durationMs": 600_000, "commandIds": [self.reader]}),
                )]
            }
            Phase::Yielded => {
                self.wakeup = Some(last_user_text(request));
                self.phase = Phase::Read;
                vec![self.call("shell_status", json!({"commandId": self.reader}))]
            }
            Phase::Read => {
                self.reader_output.push_str(&shown_output(&result));
                self.phase = Phase::Long;
                vec![exec("long", "echo long-ready; read line", 200)]
            }
            Phase::Long => {
                self.long = Some(command_id(&result));
                self.phase = Phase::LongYielded;
                vec![self.call(
                    "yield",
                    json!({"durationMs": 600_000, "commandIds": [self.long]}),
                )]
            }
            Phase::LongYielded => {
                self.stop_wakeup = Some(last_user_text(request));
                self.phase = Phase::Stopped;
                vec![self.call("shell_status", json!({"commandId": self.long}))]
            }
            Phase::Stopped => {
                self.phase = Phase::Seen;
                reply("stopped")
            }
            Phase::Seen => panic!("the flow already ended"),
        }
    }

    fn call(&mut self, tool: &str, input: serde_json::Value) -> ProviderEvent {
        self.polls += 1;
        event::tool_call(&format!("{tool}-{}", self.polls), tool, input)
    }
}

fn command_id(result: &str) -> CommandId {
    CommandId::try_from(field(result, "commandId")).unwrap()
}

/// The text of the request's last user message or steer.
fn last_user_text(request: &InferenceRequest) -> String {
    request
        .items
        .iter()
        .rev()
        .find_map(|item| match item {
            InferenceItem::UserMessage { content } | InferenceItem::UserSteer { content } => {
                content.iter().find_map(|part| match part {
                    UserPart::Text(text) => Some(text.clone()),
                    _ => None,
                })
            }
            _ => None,
        })
        .expect("the request carries a user message")
}

// Over a second: two commands run as shell jobs, which the model checks,
// feeds and waits for until the user feeds and stops them.
#[tokio::test(flavor = "local")]
async fn the_shell_tools_feed_a_waiting_command_and_a_yield_wakes_at_a_commands_end() {
    within(async {
        let model = Rc::new(RefCell::new(Model {
            phase: Phase::Started,
            reader: None,
            long: None,
            reader_output: String::new(),
            wakeup: None,
            stop_wakeup: None,
            polls: 0,
            seen: Vec::new(),
        }));
        let mut turns = vec![Turn::Events(vec![exec(
            "reader",
            "read name; read line; echo \"hello $name\"",
            200,
        )])];
        for _ in 0..100 {
            let model = model.clone();
            turns.push(Turn::Respond(Box::new(move |request| {
                model.borrow_mut().answer(request)
            })));
        }
        let script = ScriptedRuntime::new(turns);
        let fixture = Fixture::start(&script).await;
        let mut client = fixture.opened().await;

        client
            .send(ClientFrame::Send {
                message_id: "message-1".try_into().unwrap(),
                content: demi_agent_server::testing::client_text(
                    "Greet Alice, then run and stop the long command.",
                ),
            })
            .await;
        let mut frames = Vec::new();
        let mut typed = false;
        let mut stopped = false;
        while model.borrow().phase != Phase::Seen {
            frames.extend(client.next_until(is_idle).await);
            let (phase, reader, long) = {
                let model = model.borrow();
                (model.phase, model.reader.clone(), model.long.clone())
            };
            // The turn ended with a yield for the reader: the user types the
            // reader's second line, and the reader's end wakes the model.
            if phase == Phase::Yielded && !typed {
                typed = true;
                client
                    .send(ClientFrame::ShellWrite {
                        command_id: reader.unwrap(),
                        stdin: "go\n".into(),
                    })
                    .await;
            }
            // The turn ended with a yield for the long command: the user
            // stops it, and the stop wakes the model.
            if phase == Phase::LongYielded && !stopped {
                stopped = true;
                client
                    .send(ClientFrame::ShellAbort {
                        command_id: long.unwrap(),
                    })
                    .await;
            }
        }
        {
            let model = model.borrow();
            let reader = model.reader.clone().unwrap();
            let long = model.long.clone().unwrap();
            assert!(
                model.seen[0].starts_with("status: running\n"),
                "{}",
                model.seen[0]
            );
            assert!(
                model.seen[1].starts_with("status: running\n"),
                "{}",
                model.seen[1]
            );
            assert_eq!(model.reader_output, "hello Alice\n");
            // The yield's result names the command it waits for, and the
            // reader's end, not the time, woke the model.
            assert!(
                model.seen.iter().any(|seen| seen == &format!(
                    "yield scheduled\ndurationMs: 600000\ncommandIds: {reader}"
                )),
                "{:#?}",
                model.seen
            );
            assert_eq!(
                model.wakeup.as_deref(),
                Some(format!("Command {reader} ended with exit code 0. Read its end with shell_status {reader} and continue the previous work.").as_str())
            );
            assert_eq!(
                model.stop_wakeup.as_deref(),
                Some(format!("Command {long} was stopped. Read its end with shell_status {long} and continue the previous work.").as_str())
            );
            let aborted = model.seen.last().unwrap();
            assert!(aborted.starts_with("status: aborted\n"), "{aborted}");
            assert!(
                aborted.contains("next: command was intentionally stopped."),
                "{aborted}"
            );

            // The page saw each command's output and its end, and nothing of a
            // command after its end (`runtime.md` § Live output).
            let mut ends: Vec<(CommandId, ShellStatus)> = Vec::new();
            for frame in &frames {
                let ServerFrame::ShellOutput { status, .. } = frame else {
                    continue;
                };
                let command = &status.command().command_id;
                assert!(
                    !ends.iter().any(|(ended, _)| ended == command),
                    "a frame after the end of {command}"
                );
                if !matches!(**status, ShellStatus::Running { .. }) {
                    ends.push((command.clone(), ShellStatus::clone(status)));
                }
            }
            let greeted = ends.iter().find(|(command, _)| *command == reader).map(|(_, end)| end);
            assert!(
                matches!(greeted, Some(ShellStatus::Exited { exit_code: 0, command }) if command.tail == "hello Alice\n"),
                "{greeted:?}"
            );
            let stopped = ends.iter().find(|(command, _)| *command == long).map(|(_, end)| end);
            assert!(matches!(stopped, Some(ShellStatus::Aborted { .. })), "{stopped:?}");
            // No call was an error, the stop included.
            let kinds = fixture.kinds();
            assert!(
                !kinds.iter().any(|kind| kind == "tool_call:error"),
                "{kinds:?}"
            );
        }
        fixture.stop().await;
    })
    .await;
}
