//! A conversation whose Host changes, as after a target switch
//! (`sessions-and-targets.md` § Host operations): each command starts in
//! the conversation's directory on the Host it runs on, and a command's
//! handle answers only on the Host that runs it.

use demi_agent_tools::testing::shown_output;
use demi_conversation_socket_protocol::{ClientFrame, ServerFrame};
use demi_provider_common::testing::{ScriptedRuntime, Turn};

use crate::support::{Fixture, exec, reply, scripts, turn, within};

// About two seconds: four messages run a shell job each, on two Hosts.
#[tokio::test(flavor = "local")]
async fn each_command_starts_in_its_hosts_directory_and_a_handle_answers_only_on_its_host() {
    within(async {
        let (mut turns, recorded) =
            scripts(&[&["mkdir nested && cd nested && pwd"], &["pwd"], &["pwd"]]);
        turns.push(Turn::Events(vec![exec(
            "reader",
            "read name; echo \"hello $name\"",
            200,
        )]));
        turns.push(Turn::Respond(Box::new(|_| reply("waiting for a name"))));
        let fixture = Fixture::start(&ScriptedRuntime::new(turns)).await;
        let mut client = fixture.opened().await;

        let alice = fixture.work_in("alice");
        turn(&mut client, "message-1", "Make a nested directory.").await;
        let bob = fixture.work_in("bob");
        turn(&mut client, "message-2", "Where does Bob's command run?").await;
        fixture.work_in("alice");
        turn(&mut client, "message-3", "Where does Alice's command run?").await;
        let results = recorded.borrow().clone();
        let places: Vec<String> = results.iter().map(|result| shown_output(result)).collect();
        assert_eq!(
            places,
            [
                format!("{alice}/nested\n"),
                format!("{bob}\n"),
                format!("{alice}\n")
            ]
        );

        // The reader runs on Alice's Host; from Bob's, its handle is refused.
        // Its live output goes at most every quarter second (`runtime.md`
        // § Live output), so it may reach the page after the turn ended.
        let mut frames = turn(&mut client, "message-4", "Ask Alice for a name.").await;
        if !frames.iter().any(is_shell_output) {
            frames = client.next_until(is_shell_output).await;
        }
        let reader = frames
            .iter()
            .find_map(|frame| match frame {
                ServerFrame::ShellOutput { status, .. } => {
                    Some(status.command().command_id.clone())
                }
                _ => None,
            })
            .unwrap();
        fixture.work_in("bob");
        client
            .send(ClientFrame::ShellWrite {
                command_id: reader.clone(),
                stdin: "wrong\n".into(),
            })
            .await;
        let answer = client.received();
        let [ServerFrame::Error { message, .. }] = &answer[..] else {
            panic!("{answer:?}");
        };
        assert!(message.contains("belongs to a different Host"), "{message}");
        fixture.work_in("alice");
        client
            .send(ClientFrame::ShellWrite {
                command_id: reader.clone(),
                stdin: "right\n".into(),
            })
            .await;
        let answer = client
            .next_until(|frame| {
                matches!(
                    frame,
                    ServerFrame::ShellWriteResult { .. } | ServerFrame::Error { .. }
                )
            })
            .await;
        assert!(
            matches!(answer.last(), Some(ServerFrame::ShellWriteResult { .. })),
            "{answer:?}"
        );
        fixture.stop().await;
    })
    .await;
}

fn is_shell_output(frame: &ServerFrame) -> bool {
    matches!(frame, ServerFrame::ShellOutput { .. })
}
