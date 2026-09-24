//! A conversation whose Host changes, as after a target switch
//! (`sessions-and-targets.md` § Host operations): a node keeps a shell per
//! Host it used, and a command's handle answers only on the Host that runs
//! it.

use demi_agent_protocol::{ClientFrame, ServerFrame};
use demi_provider::testing::{ScriptedRuntime, Turn};

use crate::support::{Fixture, exec, preview, reply, scripts, turn, within};

#[tokio::test(flavor = "local")]
async fn a_node_keeps_each_hosts_shell_and_a_handle_answers_only_on_its_host() {
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
        turn(&mut client, "message-2", "Where is Bob's shell?").await;
        fixture.work_in("alice");
        turn(&mut client, "message-3", "Where is Alice's shell?").await;
        let results = recorded.borrow().clone();
        let places: Vec<&str> = results.iter().map(|result| preview(result)).collect();
        assert_eq!(
            places,
            [
                format!("{alice}/nested\n"),
                format!("{bob}\n"),
                format!("{alice}/nested\n")
            ]
        );

        // The reader runs on Alice's Host; from Bob's, its handle is refused.
        let frames = turn(&mut client, "message-4", "Ask Alice for a name.").await;
        let reader = frames
            .iter()
            .find_map(|frame| match frame {
                ServerFrame::ShellOutput { status } => Some(status.command().command_id.clone()),
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
        let answer = client.received();
        assert!(
            matches!(
                &answer[..],
                [
                    ServerFrame::ShellOutput { .. },
                    ServerFrame::ShellWriteResult { .. }
                ]
            ),
            "{answer:?}"
        );
        fixture.stop().await;
    })
    .await;
}
