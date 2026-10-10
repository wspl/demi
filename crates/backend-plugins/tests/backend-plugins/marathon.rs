//! The coding agent at work: the model edits files with `demi file` and
//! runs commands with the `shell` tool, on a real runner, whose ends wake
//! it.

use std::{cell::RefCell, rc::Rc};

use demi_agent_tools::testing::{field, shown_output};
use demi_conversation_socket_protocol::{ClientFrame, ServerFrame, ShellStatus};
use demi_provider_common::{
    InferenceItem, InferenceRequest, ProviderEvent, UserPart,
    testing::{ScriptedRuntime, Turn},
};
use demi_shared_types::CommandId;

use crate::support::{Fixture, is_idle, last_result, reply, resident, scripts, turn, within};

// Several seconds: five scripts over two messages run a shell job each, and
// the first `demi file` starts the `demi.file` service.
#[tokio::test(flavor = "local")]
async fn a_coding_workflow_edits_files_and_every_command_starts_in_the_workspace() {
    within(async {
        let (turns, recorded) = scripts(&[
            &[
                "demi file edit <<'EOF'\nsrc/app.ts\n<<<<<<< SEARCH\n=======\nexport const value = 1\n>>>>>>> REPLACE\nEOF",
                "grep -q 'value = 2' src/app.ts",
                "demi file edit src/app.ts --old \"1\" --new \"2\" && cd src",
                "grep -q 'value = 2' src/app.ts && echo passed",
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
        assert_eq!(shown_output(&results[0]), "Created src/app.ts (1 line)\n");
        // The failing check, then the fix, whose `cd` the passing check
        // does not inherit.
        assert_eq!(field(&results[1], "exitCode"), "1");
        assert_eq!(
            shown_output(&results[2]),
            "Edited src/app.ts (+1 \u{2212}1)\n   1  export const value = 2\n"
        );
        assert_eq!(shown_output(&results[3]), "passed\n");
        assert_eq!(
            fixture.kinds(),
            [
                "user",
                "tool_call:completed",
                // The failing check is an error to the provider.
                "tool_call:error",
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

        // A command starts in the workspace, wherever the last one went.
        turn(&mut client, "message-2", "Where are you?").await;
        let last = recorded.borrow()[4].clone();
        assert_eq!(shown_output(&last), format!("{}\n", fixture.workspace));
        assert_eq!(
            std::fs::read_to_string(format!("{}/src/app.ts", fixture.workspace)).unwrap(),
            "export const value = 2\n"
        );

        // The model was given the one tool.
        let first = &script.requests()[0];
        let tools: Vec<&str> = first.tools.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(tools, ["shell"]);
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
            "demi file\nUse it whenever you change the task's files",
            "Operations: view, edit, patch\n",
            "Details: demi file --help; one operation: demi file <operation> --help",
        ] {
            assert!(prompt.contains(taught), "{taught}");
        }
        for absent in ["Usage:", "Stdin body:", "demi file edit [<path>]", "--content"] {
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


// About five seconds: two commands run as shell jobs, each with the two
// seconds of quiet that end its call; the user feeds the one and stops the
// other.
#[tokio::test(flavor = "local")]
async fn a_page_feeds_and_stops_commands_left_running_and_their_ends_wake_the_model() {
    within(async {
        let woken: Rc<RefCell<Vec<String>>> = Rc::default();
        let results: Rc<RefCell<Vec<String>>> = Rc::default();
        let turn_of = |answer: Vec<ProviderEvent>| {
            let woken = woken.clone();
            let results = results.clone();
            Turn::Respond(Box::new(move |request: &InferenceRequest| {
                woken.borrow_mut().push(last_user_text(request));
                // A request that follows a step ends with its results.
                if matches!(request.items.last(), Some(InferenceItem::ToolResult { .. })) {
                    results.borrow_mut().push(last_result(request));
                }
                answer
            }))
        };
        let script = ScriptedRuntime::new([
            Turn::Events(vec![resident(
                "reader",
                "read name; read line; echo \"hello $name\"",
            )]),
            turn_of(reply("The reader waits.")),
            turn_of(vec![resident("long", "echo long-ready; read line")]),
            turn_of(reply("The long command runs.")),
            turn_of(reply("stopped")),
        ]);
        let fixture = Fixture::start(&script).await;
        let mut client = fixture.opened().await;

        let mut frames = turn(&mut client, "message-1", "Greet Alice.").await;
        let reader = fixture.shell_commands()[0].clone();
        // The user types the reader's two lines; its end wakes the model,
        // which starts the long command.
        for line in ["Alice\n", "go\n"] {
            client
                .send(ClientFrame::ShellWrite {
                    command_id: reader.clone(),
                    stdin: line.into(),
                })
                .await;
        }
        // Each idle ends what came before it; the last one follows the
        // model's last scripted run.
        loop {
            frames.extend(client.next_until(is_idle).await);
            if script.remaining() == 1 {
                break;
            }
        }
        // The user stops the long command; the stop wakes the model.
        let long = fixture.shell_commands()[1].clone();
        client
            .send(ClientFrame::ShellAbort {
                command_id: long.clone(),
            })
            .await;
        loop {
            frames.extend(client.next_until(is_idle).await);
            if script.remaining() == 0 {
                break;
            }
        }

        let woken = woken.borrow();
        assert_eq!(
            woken[1],
            format!(
                "Command {reader} (Run the test script) ended with exit code 0; look at it with demi shell status {reader}."
            )
        );
        assert_eq!(
            woken[3],
            format!("Command {long} (Run the test script) was stopped by the user.")
        );
        let results = results.borrow();
        assert!(results[0].starts_with("status: running\n"), "{}", results[0]);
        assert!(results[1].starts_with("status: running\n"), "{}", results[1]);
        assert_eq!(shown_output(&results[1]), "long-ready\n");

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
        fixture.stop().await;
    })
    .await;
}
