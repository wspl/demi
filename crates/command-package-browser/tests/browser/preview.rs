//! `browser.preview`, the web preview's stream, served by the program
//! (`preview.md` § The stream): one engine for the Host, whose cookie jar the
//! program keeps in its data directory. What the engine does is its own
//! crate's tests'.

use std::collections::BTreeMap;

use axum::Router;
use axum::response::IntoResponse;
use axum::routing::get;
use bytes::{Buf, BufMut, Bytes, BytesMut};
use demi_browser::DemiBrowser;
use demi_command_package_browser_protocol::preview::{
    CHUNK_FRAME, CONTROL_FRAME, PreviewClient, PreviewCredentials, PreviewEngineMessage, PreviewEnvironment,
    PreviewMode, PreviewRelayMessage, PreviewRequest, PreviewScheme,
};
use demi_command_protocol::{CommandCaller, CommandContext, CommandLocale, Invocation, Record};
use demi_command_sdk::{Handler, Input, InvocationContext, Output};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tokio_util::task::AbortOnDropHandle;

fn framed(message: &PreviewRelayMessage) -> Bytes {
    let json = serde_json::to_vec(message).unwrap();
    let mut bytes = BytesMut::new();
    bytes.put_u32(1 + json.len() as u32);
    bytes.put_u8(CONTROL_FRAME);
    bytes.put_slice(&json);
    bytes.freeze()
}

/// The engine's frames, from its invocation's records.
struct Frames {
    records: mpsc::Receiver<Record>,
    pending: BytesMut,
}

impl Frames {
    async fn next(&mut self) -> (u8, Bytes) {
        loop {
            if self.pending.len() >= 4 {
                let length = u32::from_be_bytes(self.pending[..4].try_into().unwrap()) as usize;
                if self.pending.len() >= 4 + length {
                    self.pending.advance(4);
                    let mut frame = self.pending.split_to(length).freeze();
                    return (frame.get_u8(), frame);
                }
            }
            let record = tokio::time::timeout(std::time::Duration::from_secs(10), self.records.recv())
                .await
                .expect("the engine's next frame")
                .expect("an open stream");
            let Record::Stdout(bytes) = record else {
                panic!("the stream writes only its bytes");
            };
            self.pending.extend_from_slice(&bytes);
        }
    }
}

#[tokio::test]
async fn the_program_serves_the_preview_stream_and_keeps_its_jar() {
    let data = tempfile::tempdir().unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let _site = AbortOnDropHandle::new(tokio::spawn(async move {
        let page = get(|| async { ([("set-cookie", "signed=in; Max-Age=600")], "hello").into_response() });
        axum::serve(listener, Router::new().route("/", page)).await.unwrap();
    }));
    let service = DemiBrowser::new().with_data_directory(data.path().to_owned());
    let (input, received) = mpsc::unbounded_channel::<Result<Bytes, demi_command_sdk::ServiceError>>();
    let (output, records) = Output::channel(CancellationToken::new());
    let stream = service.invoke(InvocationContext {
        request: Invocation {
            operation: "browser.preview".into(),
            invocation_id: "preview".into(),
            context: CommandContext {
                conversation: "conversation".into(),
                caller: CommandCaller::User {},
                locale: CommandLocale {
                    time_zone: "UTC".into(),
                    languages: vec!["en-US".into()],
                },
            },
            args: serde_json::json!({}),
            cwd: "/".into(),
            env: BTreeMap::new(),
            edits: None,
            json: None,
            stdout: None,
        },
        input: Input::from_stream(futures_util::stream::unfold(received, |mut received| async move {
            received.recv().await.map(|item| (item, received))
        })),
        output,
        cancellation: CancellationToken::new(),
    });
    let stream = tokio::spawn(stream);
    let mut frames = Frames {
        records,
        pending: BytesMut::new(),
    };
    let environment = PreviewEnvironment {
        origin: origin.clone(),
        top: "http://127.0.0.1".into(),
        cross: false,
    };
    for message in [
        PreviewRelayMessage::Hello {
            scheme: PreviewScheme::Https,
            domain: "preview.test".into(),
            namespace: "k3f9a2ab".into(),
            host: "host-1".into(),
        },
        PreviewRelayMessage::Request {
            id: 1,
            environment,
            request: PreviewRequest {
                url: format!("{origin}/"),
                method: "GET".into(),
                headers: Vec::new(),
                body: false,
                mode: PreviewMode::Navigate,
                destination: "document".into(),
                credentials: PreviewCredentials::Include,
                referrer: String::new(),
                referrer_policy: String::new(),
                keepalive: false,
                initiator: None,
                user: true,
            },
            client: PreviewClient {
                user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/154.0.0.0 Safari/537.36".into(),
                brands: String::new(),
                mobile: false,
                platform: "macOS".into(),
                accept_language: "en-US".into(),
            },
        },
        PreviewRelayMessage::Pull { id: 1 },
    ] {
        input.send(Ok(framed(&message))).unwrap();
    }
    let (kind, head) = frames.next().await;
    assert_eq!(kind, CONTROL_FRAME);
    assert!(matches!(
        PreviewEngineMessage::decode(&head).unwrap(),
        PreviewEngineMessage::Response { id: 1, status: 200, .. }
    ));
    let (kind, chunk) = frames.next().await;
    assert_eq!((kind, &chunk[4..]), (CHUNK_FRAME, &b"hello"[..]));
    // The page ends the stream; the program's end writes the jar.
    drop(input);
    assert_eq!(stream.await.unwrap().unwrap().exit_code, 0);
    service.close().await.unwrap();
    let jar = std::fs::read_to_string(data.path().join("preview-cookies.json")).unwrap();
    assert!(jar.contains("signed=in"), "{jar}");
}
