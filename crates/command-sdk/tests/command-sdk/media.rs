//! The media a handler returns (`commands.md` § Return media,
//! `native-runtime.md` § Response records and completion): its writer sends
//! each one as a medium record and its bytes, which the caller's exchange
//! hands on whole, in its place among the output; an invocation that is no
//! job's command cannot return one; and the bytes a medium may be.

use std::{
    collections::BTreeMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use bytes::Bytes;
use demi_command_protocol::{
    CommandCaller, CommandContext, CommandLocale, Completion, Invocation, MAX_MEDIUM_BYTES,
    MediumFacts, ProtocolError, Record, RecordDecoder, Viewable, sniff_media_type,
};
use demi_command_sdk::{
    Client, Exchange, Handler, InputSource, InvocationContext, OutputSink, ServiceError, serve,
};

/// The facts the medium's record carries to the caller, whole.
fn facts() -> MediumFacts {
    MediumFacts {
        media_type: "image/png".into(),
        name: None,
        width: Some(640),
        height: Some(480),
        duration_ms: None,
    }
}

/// A medium of three records' worth of bytes, which begins as a PNG.
fn medium() -> Bytes {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.resize(3 * 64 * 1024 + 5, 7);
    Bytes::from(bytes)
}

/// Prints a line, returns [`medium`] and prints another; a medium its
/// writer refuses it reports on stderr and goes on.
struct Returning;

impl Handler for Returning {
    type Metadata = Invocation;

    fn operations(&self) -> Vec<String> {
        vec!["shot".into()]
    }

    fn invoke(
        &self,
        context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        Box::pin(async move {
            context.output.stdout(Bytes::from_static(b"before\n")).await?;
            if let Err(refused) = context.output.medium(facts(), medium()).await {
                context
                    .output
                    .stderr(Bytes::from(refused.to_string()))
                    .await?;
            }
            context.output.stdout(Bytes::from_static(b"after\n")).await?;
            Ok(Completion {
                exit_code: 0,
                error: None,
            })
        })
    }
}

/// What reached the caller, in order.
#[derive(Debug, PartialEq)]
enum Received {
    Stdout(Bytes),
    Stderr(String),
    Medium(MediumFacts, Bytes),
}

#[derive(Default)]
struct Recorded(Vec<Received>);

impl OutputSink for Recorded {
    type Error = ServiceError;

    async fn stdout(&mut self, bytes: Bytes) -> Result<(), ServiceError> {
        self.0.push(Received::Stdout(bytes));
        Ok(())
    }

    async fn stderr(&mut self, bytes: Bytes) -> Result<(), ServiceError> {
        let text = String::from_utf8_lossy(&bytes).into_owned();
        self.0.push(Received::Stderr(text));
        Ok(())
    }

    async fn medium(&mut self, facts: MediumFacts, bytes: Bytes) -> Result<(), ServiceError> {
        self.0.push(Received::Medium(facts, bytes));
        Ok(())
    }
}

/// A caller with no input.
struct Nothing;

impl InputSource for Nothing {
    type Error = ServiceError;

    async fn next(&mut self) -> Result<Option<Bytes>, ServiceError> {
        Ok(None)
    }
}

/// Runs `shot` on the [`Returning`] service, as a job's command when
/// `viewable` names what its job may show, and answers what reached the
/// caller.
async fn shot(viewable: Option<Viewable>) -> Vec<Received> {
    let (client_io, server_io) = tokio::io::duplex(64 * 1024);
    let server = tokio::spawn(serve(server_io, Arc::new(Returning)));
    let (client, connection) = Client::connect(client_io).await.unwrap();
    let driver = tokio::spawn(connection);
    let request = Invocation {
        operation: "shot".into(),
        invocation_id: "shot".into(),
        command: "demi test".into(),
        context: CommandContext {
            color_scheme: demi_command_protocol::ColorScheme::Light,
            conversation: "conversation".into(),
            caller: CommandCaller::agent(1),
            locale: CommandLocale {
                time_zone: "UTC".into(),
                languages: vec!["en-US".into()],
            },
        },
        args: serde_json::json!({}),
        cwd: "/tmp".into(),
        env: BTreeMap::new(),
        edits: None,
        json: None,
        live_input: viewable.as_ref().map(|_| false),
        viewable,
    };
    let (input, output) = client.invoke(&request).await.unwrap();
    let mut received = Recorded::default();
    let completion = Exchange::new(input, output)
        .run(&mut Nothing, &mut received)
        .await
        .unwrap();
    assert_eq!(completion.exit_code, 0);
    drop(client);
    driver.abort();
    server.abort();
    received.0
}

/// Planted defects this catches: an exchange that hands on each record of a
/// medium rather than the whole of it, and one that hands it on after the
/// output that followed it.
#[tokio::test]
async fn a_returned_medium_reaches_the_caller_whole_in_its_place() {
    let received = tokio::time::timeout(Duration::from_secs(5), shot(Some(Viewable {
        model: "test-model".into(),
        media_types: Some(vec!["image/png".into()]),
    })))
        .await
        .unwrap();
    assert_eq!(
        received,
        [
            Received::Stdout(Bytes::from_static(b"before\n")),
            Received::Medium(facts(), medium()),
            Received::Stdout(Bytes::from_static(b"after\n")),
        ]
    );
}

/// Planted defect this catches: a writer that sends media for every
/// invocation, as a user stream's or a package call's would then.
#[tokio::test]
async fn the_writer_of_an_invocation_that_is_no_jobs_command_refuses_media() {
    let received = tokio::time::timeout(Duration::from_secs(5), shot(None))
        .await
        .unwrap();
    assert_eq!(
        received,
        [
            Received::Stdout(Bytes::from_static(b"before\n")),
            Received::Stderr("this invocation cannot return media: only a job's command can".into()),
            Received::Stdout(Bytes::from_static(b"after\n")),
        ]
    );
}

#[test]
fn a_medium_over_16_mib_breaks_the_protocol() {
    let payload = format!(
        "{{\"size\":{},\"mediaType\":\"image/png\"}}",
        MAX_MEDIUM_BYTES + 1
    );
    let mut record = vec![5];
    record.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    record.extend_from_slice(payload.as_bytes());
    assert!(matches!(
        RecordDecoder::default().decode(&mut record.into()),
        Err(ProtocolError::TooLarge)
    ));
    let largest = Record::Medium {
        size: MAX_MEDIUM_BYTES,
        facts: facts(),
    };
    let mut encoded = largest.encode().unwrap();
    assert_eq!(
        RecordDecoder::default().decode(&mut encoded).unwrap(),
        Some(largest)
    );
}

/// `parts` joined and padded with zeros to 16 bytes.
fn bytes(parts: &[&[u8]]) -> Vec<u8> {
    let mut bytes = parts.concat();
    bytes.resize(bytes.len().max(16), 0);
    bytes
}

#[test]
fn media_are_recognized_by_their_magic_numbers_and_nothing_else() {
    let sniffed = sniff_media_type;
    assert_eq!(sniffed(&bytes(&[b"\x89PNG\r\n\x1a\n"])), Some("image/png"));
    assert_eq!(sniffed(&bytes(&[b"\xff\xd8\xff\xe0"])), Some("image/jpeg"));
    assert_eq!(sniffed(&bytes(&[b"GIF89a"])), Some("image/gif"));
    assert_eq!(
        sniffed(&bytes(&[b"RIFF", b"\x01\x02\x03\x04", b"WEBP"])),
        Some("image/webp")
    );
    assert_eq!(sniffed(&bytes(&[b"\x1a\x45\xdf\xa3"])), Some("video/webm"));
    assert_eq!(
        sniffed(&bytes(&[b"\0\0\0\x20", b"ftypisom"])),
        Some("video/mp4")
    );
    assert_eq!(
        sniffed(&bytes(&[b"\0\0\0\x20", b"ftypqt  "])),
        Some("video/quicktime")
    );
    assert_eq!(
        sniffed(&bytes(&[b"\0\0\0\x20", b"ftypM4V "])),
        Some("video/x-m4v")
    );

    assert_eq!(sniffed(&bytes(&[b"%PDF-1.7"])), Some("application/pdf"));

    // Outside the closed set, and too short to tell: no guessing.
    assert_eq!(sniffed(&bytes(&[b"plain text here"])), None);
    assert_eq!(sniffed(b"\x89PNG\r\n\x1a\n\0\0\0"), None);
}
