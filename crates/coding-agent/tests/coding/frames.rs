//! The live output of commands (`runtime.md` § Live output): every attached
//! page receives each command's output as it comes and its end, with no frame
//! of its own; a page that attaches finds the live commands in its
//! handshake and then follows them too; a page writes to a command and stops
//! it; a chatty command fills no page's outbox; and the model's place in the
//! output stays its own.

use std::{cell::RefCell, rc::Rc, time::Duration, time::Instant};

use demi_agent::{
    ServerConfig,
    testing::{TestClient, shown_output},
};
use demi_agent_protocol::{ClientFrame, ServerFrame, ShellStatus, TranscriptPatch};
use demi_core::{Block, CommandId, ToolCallStatus};
use demi_provider::testing::{ScriptedRuntime, Turn, event};
use serde_json::json;

use crate::support::{Fixture, Harness, exec, last_result, reply, turn, within};

/// The command views among `frames`, in order.
fn shell_outputs(frames: &[ServerFrame]) -> Vec<ShellStatus> {
    frames
        .iter()
        .filter_map(|frame| match frame {
            ServerFrame::ShellOutput { status, .. } => Some(ShellStatus::clone(status)),
            _ => None,
        })
        .collect()
}

fn command_of(status: &ShellStatus) -> &CommandId {
    &status.command().command_id
}

fn is_running(status: &ShellStatus) -> bool {
    matches!(status, ShellStatus::Running { .. })
}

/// Whether `frame` completes the call `tool_use_id` with its result.
fn completes(frame: &ServerFrame, tool_use_id: &str) -> bool {
    let ServerFrame::TranscriptPatch { patches, .. } = frame else {
        return false;
    };
    patches.iter().any(|patch| {
        matches!(
            patch,
            TranscriptPatch::ReplaceBlock { value: Block::ToolCall(call), .. }
                if call.tool_use_id == tool_use_id && call.status != ToolCallStatus::Executing
        )
    })
}

/// Reads `client` until a view of `command` satisfies `wanted`, and returns
/// it: the page asks for nothing.
async fn view_until(
    client: &mut TestClient<Harness>,
    command: &CommandId,
    wanted: impl Fn(&ShellStatus) -> bool,
) -> ShellStatus {
    loop {
        let frame = client.next().await.expect("the page stays attached");
        if let ServerFrame::ShellOutput { status, .. } = frame
            && command_of(&status) == command
            && wanted(&status)
        {
            return *status;
        }
    }
}

// Over a second: four commands run as shell jobs.
#[tokio::test(flavor = "local")]
async fn every_page_sees_a_commands_output_as_it_comes_and_its_end() {
    within(async {
        let script = ScriptedRuntime::new([
            Turn::Events(vec![exec("greeter", "echo hello", 30_000)]),
            Turn::Respond(Box::new(|_| reply("greeted"))),
            Turn::Events(vec![exec("reader", "read name; echo \"hello $name\"", 200)]),
            Turn::Respond(Box::new(|_| reply("waiting for a name"))),
            Turn::Events(vec![exec(
                "long",
                "echo long-ready; while [ ! -e go ]; do sleep 0.02; done; echo went; sleep 30",
                200,
            )]),
            Turn::Respond(Box::new(|_| reply("waiting"))),
            Turn::Events(vec![exec(
                "sleeper",
                "sh -c 'echo $$ > ../sleeper.pid; exec sleep 30'",
                200,
            )]),
            Turn::Respond(Box::new(|_| reply("sleeping"))),
        ]);
        let fixture = Fixture::start(&script).await;
        let mut client = fixture.opened().await;

        // A command's output reaches the page under its call, before the
        // call's result: here with its end, since it ends within its window.
        let frames = turn(&mut client, "message-1", "Greet.").await;
        let greeted = frames
            .iter()
            .position(|frame| {
                matches!(frame, ServerFrame::ShellOutput { status, .. }
                    if matches!(**status, ShellStatus::Exited { exit_code: 0, .. })
                        && status.command().tail == "hello\n")
            })
            .unwrap_or_else(|| panic!("the greeter's end: {frames:?}"));
        let returned = frames
            .iter()
            .position(|frame| completes(frame, "greeter"))
            .expect("the call's result");
        assert!(greeted < returned, "{frames:?}");
        let ServerFrame::ShellOutput {
            subagent_id,
            status,
        } = &frames[greeted]
        else {
            unreachable!("found above");
        };
        assert_eq!(
            (subagent_id, status.command().tool_use_id.as_str()),
            (&None, "greeter")
        );

        // A page's write is answered alone; what the command then prints and
        // its end come by themselves.
        turn(&mut client, "message-2", "Ask for a name.").await;
        let reader = fixture.shell_commands().last().unwrap().clone();
        client
            .send(ClientFrame::ShellWrite {
                command_id: reader.clone(),
                stdin: "Alice\n".into(),
            })
            .await;
        let answer = client
            .next_until(|frame| matches!(frame, ServerFrame::ShellWriteResult { .. }))
            .await;
        assert!(
            !answer
                .iter()
                .any(|frame| matches!(frame, ServerFrame::Error { .. })),
            "{answer:?}"
        );
        let answered = shell_outputs(&answer)
            .into_iter()
            .find(|status| command_of(status) == &reader && !is_running(status));
        let end = match answered {
            Some(end) => end,
            None => view_until(&mut client, &reader, |status| !is_running(status)).await,
        };
        assert!(
            matches!(&end, ShellStatus::Exited { exit_code: 0, command } if command.tail == "hello Alice\n"),
            "{end:?}"
        );

        // A page that attaches while a command runs finds it in its
        // handshake, whatever the other page read, beside the ended reader
        // that the transcript last showed running; then it follows as the
        // other page does.
        turn(&mut client, "message-3", "Wait.").await;
        let long = fixture.shell_commands().last().unwrap().clone();
        view_until(&mut client, &long, |status| {
            status.command().tail == "long-ready\n"
        })
        .await;
        let mut second = fixture.attach().await;
        let handshake = second.received();
        let live: Vec<(CommandId, bool, String)> = shell_outputs(&handshake)
            .iter()
            .map(|status| {
                (
                    command_of(status).clone(),
                    is_running(status),
                    status.command().tail.clone(),
                )
            })
            .collect();
        assert_eq!(
            live,
            [
                (reader.clone(), false, "hello Alice\n".to_owned()),
                (long.clone(), true, "long-ready\n".to_owned())
            ]
        );
        assert!(
            matches!(handshake.last(), Some(ServerFrame::ShellOutput { .. })),
            "{handshake:?}"
        );
        // The command prints after its call returned: both pages see it.
        std::fs::write(format!("{}/go", fixture.workspace), "").unwrap();
        for page in [&mut client, &mut second] {
            view_until(page, &long, |status| {
                status.command().tail.ends_with("went\n")
            })
            .await;
        }

        // A stop from one page shows its end on both; a write to the command
        // then is refused to the page that wrote.
        second
            .send(ClientFrame::ShellAbort {
                command_id: long.clone(),
            })
            .await;
        for page in [&mut client, &mut second] {
            let stopped = view_until(page, &long, |status| !is_running(status)).await;
            assert!(
                matches!(stopped, ShellStatus::Aborted { .. }),
                "{stopped:?}"
            );
        }
        second
            .send(ClientFrame::ShellWrite {
                command_id: long.clone(),
                stdin: "late\n".into(),
            })
            .await;
        let answer = second.received();
        assert!(
            matches!(&answer[..], [ServerFrame::Error { .. }]),
            "{answer:?}"
        );

        // Closing the conversation stops the commands its shells still run:
        // the sleeper's process ends. The turn ends when the exec's window
        // does, which can come before the sleeper wrote its process id: its
        // job first reads the machine's system profile (`runner.md` § Shell
        // jobs). Nothing of an ended command follows its end.
        let frames = turn(&mut second, "message-4", "Start the sleeper.").await;
        let written = format!("{}/sleeper.pid", fixture.runner.home());
        let sleeper = loop {
            // The line is whole once it ends with its newline.
            let pid = std::fs::read_to_string(&written)
                .ok()
                .and_then(|line| line.strip_suffix('\n')?.parse::<i32>().ok());
            if let Some(pid) = pid {
                break rustix::process::Pid::from_raw(pid).expect("a process id is positive");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        };
        second.send(ClientFrame::Close {}).await;
        let closing = second.received();
        assert_eq!(closing.last(), Some(&ServerFrame::Closed));
        let later: Vec<ServerFrame> = frames.into_iter().chain(closing).collect();
        assert!(
            !shell_outputs(&later)
                .iter()
                .any(|status| [&reader, &long].contains(&command_of(status))),
            "{later:?}"
        );
        assert_eq!(client.received().last(), Some(&ServerFrame::Closed));
        let ended = tokio::time::timeout(Duration::from_secs(10), async {
            while rustix::process::test_kill_process(sleeper).is_ok() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        assert!(ended.is_ok(), "the sleeper still runs after the close");
        fixture.stop().await;
    })
    .await;
}

/// The model keeps its own place in a command's output: the page seeing a
/// running command's new output leaves all of it to the model's next
/// `shell_status` (`runtime.md` § Results and previews).
// Over a second: the command runs as a shell job.
#[tokio::test(flavor = "local")]
async fn what_a_page_sees_of_a_running_command_is_left_to_the_model() {
    within(async {
        let command: Rc<RefCell<Option<CommandId>>> = Rc::default();
        let checked: Rc<RefCell<String>> = Rc::default();
        let asked = command.clone();
        let seen = checked.clone();
        let script = ScriptedRuntime::new([
            // The output waits for the page's write, so nothing of it is in
            // the exec's result.
            Turn::Events(vec![exec("later", "read go; echo later; read line", 100)]),
            Turn::Respond(Box::new(|_| reply("started"))),
            Turn::Respond(Box::new(move |_| {
                vec![event::tool_call(
                    "check",
                    "shell_status",
                    json!({"commandId": asked.borrow().clone().expect("the command started")}),
                )]
            })),
            Turn::Respond(Box::new(move |request| {
                *seen.borrow_mut() = last_result(request);
                reply("checked")
            })),
        ]);
        let fixture = Fixture::start(&script).await;
        let mut client = fixture.opened().await;
        turn(&mut client, "message-1", "Start it.").await;
        let started = fixture.shell_commands()[0].clone();
        *command.borrow_mut() = Some(started.clone());

        // The page lets the output come and sees it as it comes.
        client
            .send(ClientFrame::ShellWrite {
                command_id: started.clone(),
                stdin: "go\n".into(),
            })
            .await;
        view_until(&mut client, &started, |status| {
            status.command().tail == "later\n"
        })
        .await;

        // The model's look still shows all of it.
        turn(&mut client, "message-2", "Check it.").await;
        let result = checked.borrow().clone();
        assert!(shown_output(&result).contains("later"), "{result}");

        client
            .send(ClientFrame::ShellAbort {
                command_id: started,
            })
            .await;
        client.received();
        fixture.stop().await;
    })
    .await;
}

/// A command that prints faster than a page reads sends each page at most
/// one frame every 250 ms, so a page that reads nothing until the end keeps
/// its connection; and while a page is attached the runner follows the
/// command, so its view shows output beyond the first 32 KiB of a stream
/// while it runs (`runtime.md` § Live output). About two seconds: a login
/// shell, a loop of 1,000 writes, which a page's outbox of 64 frames would
/// not hold one frame each, then 140 KB of numbers.
#[tokio::test(flavor = "local")]
async fn a_chatty_command_fills_no_outbox_and_shows_output_beyond_its_first_32_kib() {
    within(async {
        let script = ScriptedRuntime::new([
            Turn::Events(vec![exec(
                "chatty",
                "i=0; while [ $i -lt 1000 ]; do echo $i; i=$((i+1)); done; seq 100000 120000; echo beyond; while [ ! -e done ]; do sleep 0.02; done",
                200,
            )]),
            Turn::Respond(Box::new(|_| reply("chatting"))),
        ]);
        let config = ServerConfig {
            outbox_frames: 64,
            ..ServerConfig::default()
        };
        let fixture = Fixture::start_with(&script, config).await;
        let mut client = fixture.opened().await;
        // A page that reads nothing until the command ended.
        let mut idle = fixture.attach().await;
        let started = Instant::now();
        turn(&mut client, "message-1", "Chat.").await;
        let chatty = fixture.shell_commands()[0].clone();
        // `beyond` comes after 140 KB of numbers, far past the first 32 KiB.
        view_until(&mut client, &chatty, |status| {
            is_running(status) && status.command().tail.contains("beyond\n")
        })
        .await;
        std::fs::write(format!("{}/done", fixture.workspace), "").unwrap();
        view_until(&mut client, &chatty, |status| !is_running(status)).await;
        let elapsed = started.elapsed();
        let views: Vec<ShellStatus> = shell_outputs(&idle.received())
            .into_iter()
            .filter(|status| command_of(status) == &chatty)
            .collect();
        assert!(
            matches!(views.last(), Some(ShellStatus::Exited { .. })),
            "the page that read nothing kept its connection: {:?}",
            views.last()
        );
        let intervals = elapsed.as_millis() / 250;
        assert!(
            views.len() as u128 <= intervals + 2,
            "{} frames in {elapsed:?}",
            views.len()
        );
        fixture.stop().await;
    })
    .await;
}
