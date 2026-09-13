use demi_command_service::protocol::Invocation;
use demi_runner::connection::wire::{HelloRunner, HelloRunnerIdentity, JobExitOutput, WireBytes};
use demi_runner::{
    commands::command_client::{Stdio, forward},
    mode::{self, Options},
    state::RunnerState,
};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{WebSocketStream, tungstenite::Message};
use tokio_util::sync::CancellationToken;

#[derive(serde::Deserialize)]
#[serde(tag = "type")]
enum Reply {
    #[serde(rename = "hello")]
    Hello { protocol: f64 },
    #[serde(rename = "job_output")]
    Output { stream: String, bytes: WireBytes },
    #[serde(rename = "job_exit")]
    Exit {
        #[serde(rename = "exitCode")]
        code: Option<f64>,
        output: Option<JobExitOutput>,
    },
}

async fn send(socket: &mut WebSocketStream<TcpStream>, value: Value) {
    socket
        .send(Message::Binary(
            rmp_serde::to_vec_named(&value).unwrap().into(),
        ))
        .await
        .unwrap();
}
async fn receive(socket: &mut WebSocketStream<TcpStream>) -> Reply {
    let message = socket.next().await.unwrap().unwrap();
    rmp_serde::from_slice(&message.into_data()).unwrap()
}

#[tokio::test]
async fn backend_job_invokes_same_binary_alias_and_drain_releases_installation() {
    tokio::time::timeout(Duration::from_secs(20), async {
        let directory = tempfile::tempdir().unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let backend = format!("http://{}", listener.local_addr().unwrap());
        let state_dir = directory.path().join("state");
        let home = directory.path().to_string_lossy().into_owned();
        let options = Options {
            backend,
            directory: state_dir.clone(),
            executable: env!("CARGO_BIN_EXE_demi-runner").into(),
            cwd: directory.path().into(),
            env: BTreeMap::from([("HOME".into(), home.clone())]),
            token: Some("test-token".into()),
            volumes: vec![],
            runner: HelloRunner {
                name: "test".into(),
                platform: "test".into(),
                version: "test".into(),
                managed: None,
                identity: HelloRunnerIdentity {
                    uid: 1000.0,
                    gid: 1000.0,
                    hostname: "test".into(),
                    home_dir: home.clone(),
                },
            },
        };
        let stop = CancellationToken::new();
        let _guard = stop.clone().drop_guard();
        let mut running = tokio::spawn(mode::run(options, stop));
        let (socket, _) = tokio::select! {
            accepted = listener.accept() => accepted.unwrap(),
            result = &mut running => panic!("runner exited before backend connection: {result:?}"),
        };
        let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
        match receive(&mut socket).await {
            Reply::Hello { protocol } => {
                assert_eq!(protocol, demi_runner::connection::wire::VERSION as f64)
            }
            _ => panic!("expected hello"),
        }
        send(&mut socket, json!({"type":"hello_ok", "deviceId":"device"})).await;
        eprintln!("mode test: runner connected");
        let body = json!({"roots": {"fixture": {"tree": {
            "name":"fixture", "summary":"Remote declaration", "kind":"rpc", "stdinField":"body",
            "input":{"type":"object", "properties":{"body":{"type":"string"}}, "required":["body"]}
        }}}, "packages":{}});
        let hash = demi_command_service::protocol::canonical_digest(&body).unwrap();
        let mut manifest = body;
        manifest["hash"] = hash.clone().into();
        send(&mut socket, json!({"type":"manifest", "manifest":manifest})).await;
        // No stdin EOF is sent: --help must complete without waiting for input.
        send(
            &mut socket,
            json!({"type":"job_start", "jobId":"job", "manifestHash":hash,
            "script":"fixture --help && printf done", "cwd":home, "env":{}}),
        )
        .await;
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        loop {
            match receive(&mut socket).await {
                Reply::Output { stream, bytes } => {
                    if stream == "stdout" {
                        stdout.extend(bytes.0);
                    } else {
                        stderr.extend(bytes.0);
                    }
                }
                Reply::Exit { code, output } => {
                    assert_eq!(
                        code,
                        Some(0.0),
                        "stderr={}",
                        String::from_utf8_lossy(&stderr)
                    );
                    assert_eq!(std::fs::read(output.unwrap().stdout_path).unwrap(), stdout);
                    break;
                }
                _ => panic!("unexpected runner reply"),
            }
        }
        assert!(
            String::from_utf8(stdout)
                .unwrap()
                .contains("fixture: Remote declaration")
        );
        eprintln!("mode test: alias job completed");
        let state = Arc::new(RunnerState::open(state_dir.clone()).await.unwrap());
        let active = state.active().await.unwrap();
        assert!(state.lock().is_err());
        let request = Invocation {
            operation: "manage".into(),
            invocation_id: "drain".into(),
            args: json!({"secret":active.secret, "action":"drain"}),
            cwd: home,
            env: BTreeMap::new(),
        };
        let completion = forward(
            &active.endpoint,
            &request,
            Stdio {
                stdin: tokio::io::empty(),
                stdout: tokio::io::sink(),
                stderr: tokio::io::sink(),
            },
            CancellationToken::new(),
        )
        .await
        .unwrap();
        assert_eq!(completion.exit_code, 0);
        eprintln!("mode test: drain acknowledged");
        running.await.unwrap().unwrap();
        assert!(!state_dir.join("active.json").exists());
        state.lock().unwrap().release().unwrap();
    })
    .await
    .unwrap();
}
