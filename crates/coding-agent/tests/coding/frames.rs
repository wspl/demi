//! The shell frames (`runtime.md` § Frame protocol): a client sees the
//! root's running commands, writes to them and stops them, and finds the
//! ones still owned when it attaches or asks for a fresh transcript.

use std::time::Duration;

use demi_agent_protocol::{ClientFrame, ServerFrame, ShellStatus};
use demi_core::CommandId;
use demi_provider::testing::{ScriptedRuntime, Turn};

use crate::support::{Fixture, exec, reply, turn, within};

/// The statuses among `frames`, in order.
fn shell_outputs(frames: &[ServerFrame]) -> Vec<ShellStatus> {
    frames
        .iter()
        .filter_map(|frame| match frame {
            ServerFrame::ShellOutput { status } => Some(ShellStatus::clone(status)),
            _ => None,
        })
        .collect()
}

fn command_of(status: &ShellStatus) -> &CommandId {
    &status.command().command_id
}

#[tokio::test(flavor = "local")]
async fn a_client_sees_the_roots_live_commands_writes_to_them_and_stops_them() {
    within(async {
        let script = ScriptedRuntime::new([
            Turn::Events(vec![exec("reader", "read name; echo \"hello $name\"", 200)]),
            Turn::Respond(Box::new(|_| reply("waiting for a name"))),
            Turn::Events(vec![exec("long", "sleep 30", 200)]),
            Turn::Respond(Box::new(|_| reply("sleeping"))),
            Turn::Events(vec![exec(
                "ticker",
                "while true; do echo tick >> ../ticks.txt; sleep 0.05; done",
                200,
            )]),
            Turn::Respond(Box::new(|_| reply("ticking"))),
        ]);
        let fixture = Fixture::start(&script).await;
        let mut client = fixture.opened().await;

        // The exec's status reaches the client while the turn runs.
        let frames = turn(&mut client, "message-1", "Ask for a name.").await;
        let outputs = shell_outputs(&frames);
        assert_eq!(outputs.len(), 1, "{outputs:?}");
        assert!(
            matches!(outputs[0], ShellStatus::Running { .. }),
            "{outputs:?}"
        );
        let reader = command_of(&outputs[0]).clone();

        // A write answers the command's status, then its acknowledgement.
        client
            .send(ClientFrame::ShellWrite {
                command_id: reader.clone(),
                stdin: "Alice\n".into(),
            })
            .await;
        let answer = client.received();
        let [
            ServerFrame::ShellOutput { status },
            ServerFrame::ShellWriteResult { command_id },
        ] = &answer[..]
        else {
            panic!("{answer:?}");
        };
        assert_eq!(command_of(status), &reader);
        assert_eq!(command_id, &reader);
        let mut output = status.command().output.text.clone();

        // A fresh transcript shows the reader until it ended.
        loop {
            client.send(ClientFrame::SyncTranscript {}).await;
            let frames = client.received();
            assert!(
                matches!(frames.first(), Some(ServerFrame::TranscriptReset { .. })),
                "{frames:?}"
            );
            let outputs = shell_outputs(&frames);
            assert_eq!(outputs.len(), 1, "{outputs:?}");
            assert_eq!(command_of(&outputs[0]), &reader);
            output.push_str(&outputs[0].command().output.text);
            if let ShellStatus::Exited { exit_code, .. } = outputs[0] {
                assert_eq!(exit_code, 0);
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(output, "hello Alice\n");

        let frames = turn(&mut client, "message-2", "Sleep.").await;
        let long = command_of(&shell_outputs(&frames)[0]).clone();

        // A client that attaches finds, after the snapshot, each command the
        // transcript last saw running that the root still owns, in order.
        let mut second = fixture.attach().await;
        let handshake = second.received();
        let outputs = shell_outputs(&handshake);
        assert!(
            matches!(handshake.last(), Some(ServerFrame::ShellOutput { .. })),
            "{handshake:?}"
        );
        assert_eq!(outputs.len(), 2, "{outputs:?}");
        assert!(
            matches!(outputs[0], ShellStatus::Exited { .. }),
            "{outputs:?}"
        );
        assert_eq!(command_of(&outputs[0]), &reader);
        assert!(
            matches!(outputs[1], ShellStatus::Running { .. }),
            "{outputs:?}"
        );
        assert_eq!(command_of(&outputs[1]), &long);

        // A stop answers the stopped command's status; a write to a command
        // that no longer runs answers an error.
        second
            .send(ClientFrame::ShellAbort {
                command_id: long.clone(),
            })
            .await;
        let answer = second.received();
        let [ServerFrame::ShellOutput { status }] = &answer[..] else {
            panic!("{answer:?}");
        };
        assert!(
            matches!(**status, ShellStatus::Aborted { .. }),
            "{status:?}"
        );
        assert_eq!(command_of(status), &long);
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
        // the ticker's file stops growing.
        turn(&mut second, "message-3", "Start the ticker.").await;
        second.send(ClientFrame::Close {}).await;
        assert_eq!(second.received().last(), Some(&ServerFrame::Closed));
        let ticks = format!("{}/ticks.txt", fixture.runner.home());
        let size = || std::fs::metadata(&ticks).unwrap().len();
        let stopped = size();
        assert!(stopped > 0);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(size(), stopped);
        fixture.stop().await;
    })
    .await;
}
