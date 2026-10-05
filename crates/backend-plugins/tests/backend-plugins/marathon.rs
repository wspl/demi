//! The coding agent at work: the model edits files with `demi file` and
//! controls running commands with the shell tools, on a real runner.

use std::{cell::RefCell, rc::Rc};

use demi_agent_tools::testing::{field, shown_output};
use demi_conversation_socket_protocol::{ClientFrame, ServerFrame, ShellStatus};
use demi_provider_common::{
    InferenceRequest, ProviderEvent,
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

        // The model was given the five tools.
        let first = &script.requests()[0];
        let tools: Vec<&str> = first.tools.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(
            tools,
            [
                "shell_exec",
                "shell_status",
                "shell_write",
                "shell_abort",
                "yield"
            ]
        );
        // The prompt teaches the node's commands: the plugins' `demi` root
        // with the `demi agent` graft beside `file`, and bodies only from
        // stdin.
        let prompt = &first.system_prompt;
        for taught in [
            "You are a coding agent.",
            "Registered commands:",
            "demi file create",
            "demi agent spawn",
            "demi agent abort",
            "demi agent list",
            "demi agent show",
            "demi agent send <id> [--json] <<'EOF'",
            "Stdin body: content",
            "Stdin body: patch",
            "Stdin body: prompt",
            "Stdin body: message",
            "cannot see this conversation",
            "State the exact shape of the last assistant text it should return.",
            "demi agent profiles",
            // The help names no profile, since the user's profiles change
            // while a tree is open (`subagents.md` § Command help).
            "The name of one of the user's subagent profiles",
        ] {
            assert!(prompt.contains(taught), "{taught}");
        }
        for absent in [
            "demi agent steer",
            "--content",
            "--patch",
            "--prompt",
            "--message",
        ] {
            assert!(!prompt.contains(absent), "{absent}");
        }
        fixture.stop().await;
    })
    .await;
}

/// Where the model is in the flow below.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// The reader runs; the model checks it once.
    Started,
    /// The reader waits for input; the model writes it.
    Checked,
    /// The model waits for the reader to end, yielding between checks.
    Reading,
    /// The long command runs; the model stops it.
    Long,
    /// The long command was stopped; the model ends the turn.
    Stopped,
    Done,
}

/// The model's side of the flow: it answers each request from the result
/// it carries, and records what it saw for the test to check.
struct Model {
    phase: Phase,
    reader: Option<CommandId>,
    long: Option<CommandId>,
    /// The reader's output, from every result that showed some.
    reader_output: String,
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
                self.phase = Phase::Reading;
                vec![self.call(
                    "shell_write",
                    json!({"commandId": self.reader, "stdin": "Alice\n", "description": "Answer with the name"}),
                )]
            }
            Phase::Reading => {
                if result.starts_with("yield scheduled") {
                    return vec![self.call("shell_status", json!({"commandId": self.reader}))];
                }
                self.reader_output.push_str(&shown_output(&result));
                if field(&result, "status") == "running" {
                    return vec![self.call("yield", json!({"durationMs": 20}))];
                }
                self.phase = Phase::Long;
                vec![exec("long", "echo long-ready; sleep 30", 200)]
            }
            Phase::Long => {
                self.long = Some(command_id(&result));
                self.phase = Phase::Stopped;
                vec![self.call("shell_abort", json!({"commandId": self.long}))]
            }
            Phase::Stopped => {
                self.phase = Phase::Done;
                reply("stopped")
            }
            Phase::Done => panic!("the flow already ended"),
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

// Over a second: two commands run as shell jobs, which the model checks,
// feeds and stops.
#[tokio::test(flavor = "local")]
async fn the_shell_tools_feed_a_waiting_command_stop_a_long_one_and_yield_between_checks() {
    within(async {
        let model = Rc::new(RefCell::new(Model {
            phase: Phase::Started,
            reader: None,
            long: None,
            reader_output: String::new(),
            polls: 0,
            seen: Vec::new(),
        }));
        let mut turns = vec![Turn::Events(vec![exec(
            "reader",
            "read name; echo \"hello $name\"",
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
        while model.borrow().phase != Phase::Done {
            frames.extend(client.next_until(is_idle).await);
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
            let [(first, greeted), (second, stopped)] = &ends[..] else {
                panic!("{ends:?}");
            };
            assert_eq!((first, second), (&reader, &long));
            assert!(
                matches!(greeted, ShellStatus::Exited { exit_code: 0, command } if command.tail == "hello Alice\n"),
                "{greeted:?}"
            );
            assert!(matches!(stopped, ShellStatus::Aborted { .. }), "{stopped:?}");
            // No call was an error, the abort included.
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
