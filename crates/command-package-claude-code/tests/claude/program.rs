#![cfg(unix)]
//! The `demi-claude-code` program at its boundary (`claude-code.md` § The
//! package): the command service it serves on its standard input and
//! output, as a runner starts it, answers each invocation with one document
//! and says a failure in its completion too.
//! As an integration test of the package, it also makes `cargo test` build
//! the program, which the backend's scenarios start. `HOME` names the user's
//! root on Unix only.

use std::path::{Path, PathBuf};

use bytes::Bytes;
use demi_command_protocol::{CommandCaller, CommandContext, CommandLocale, Completion, Invocation};
use demi_command_sdk::{Exchange, Input, OutputSink, testing::ServiceProcess};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const BODY: &[u8] = b"#!/bin/sh\necho fixture claude\n";

/// What the service wrote to its standard output and error.
#[derive(Default)]
struct Written {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

impl OutputSink for Written {
    type Error = std::convert::Infallible;

    async fn stdout(&mut self, bytes: Bytes) -> Result<(), Self::Error> {
        self.stdout.extend_from_slice(&bytes);
        Ok(())
    }

    async fn stderr(&mut self, bytes: Bytes) -> Result<(), Self::Error> {
        self.stderr.extend_from_slice(&bytes);
        Ok(())
    }
}

/// Invokes `operation` with `input` as its standard input, and returns the
/// document the service wrote and its completion.
async fn invoke(service: &ServiceProcess, operation: &str, input: Vec<u8>) -> (Value, Completion) {
    let (writer, output) = service
        .client()
        .invoke(&Invocation {
            operation: operation.into(),
            invocation_id: "invocation".into(),
            context: CommandContext {
                conversation: "conversation".into(),
                caller: CommandCaller::agent(1),
                locale: CommandLocale {
                    time_zone: "UTC".into(),
                    languages: vec!["en-US".into()],
                },
            },
            args: json!({}),
            cwd: "/".into(),
            env: Default::default(),
            edits: None,
            json: None,
        })
        .await
        .unwrap();
    // The service reads its input a record at a time.
    let chunks: Vec<_> = input
        .chunks(64 * 1024)
        .map(|chunk| Ok(Bytes::copy_from_slice(chunk)))
        .collect();
    let mut source = Input::from_stream(futures_util::stream::iter(chunks));
    let mut written = Written::default();
    let completion = Exchange::new(writer, output)
        .run(&mut source, &mut written)
        .await
        .unwrap();
    assert_eq!(written.stderr, b"", "{operation}");
    assert_eq!(written.stdout.pop(), Some(b'\n'), "{operation}");
    (serde_json::from_slice(&written.stdout).unwrap(), completion)
}

/// A release record of `version` whose entry for this machine is `url`.
fn record(version: &str, url: &str) -> Vec<u8> {
    let platform = demi_claude_code::platform::current().unwrap();
    let sha256 = format!("{:x}", Sha256::digest(BODY));
    json!({ "version": version, "platforms": { platform: { "url": url, "size": BODY.len(), "sha256": sha256 } } })
        .to_string()
        .into_bytes()
}

/// A verified installation of `version` under `home`, with its receipt, as
/// `claude-code.ensure` leaves it: the service answers from it without a
/// download.
fn installed(home: &Path, version: &str) -> PathBuf {
    let directory = home.join(".demi/claude").join(version);
    std::fs::create_dir_all(&directory).unwrap();
    let executable = directory.join("claude");
    std::fs::write(&executable, BODY).unwrap();
    let receipt = json!({
        "version": version,
        "platform": demi_claude_code::platform::current().unwrap(),
        "sha256": format!("{:x}", Sha256::digest(BODY)),
        "size": BODY.len(),
    });
    std::fs::write(directory.join("receipt.json"), receipt.to_string()).unwrap();
    executable
}

#[tokio::test]
async fn the_service_answers_one_document_for_each_invocation() {
    let home = tempfile::tempdir().unwrap();
    let path = installed(home.path(), "2.1.278");
    let service = ServiceProcess::start(
        env!("CARGO_BIN_EXE_demi-claude-code"),
        &["--command-service"],
        &[("HOME", home.path().to_str().unwrap())],
    )
    .await
    .unwrap();
    let info = service.client().info().await.unwrap();
    assert_eq!(info.operations, ["claude-code.ensure", "claude-code.status"]);

    let (document, completion) = invoke(
        &service,
        "claude-code.ensure",
        record("2.1.278", "https://downloads.example.test/claude"),
    )
    .await;
    assert_eq!(
        document,
        json!({ "ok": true, "version": "2.1.278", "path": path })
    );
    assert_eq!((completion.exit_code, completion.error), (0, None));

    let (document, completion) = invoke(&service, "claude-code.status", Vec::new()).await;
    assert_eq!(
        document,
        json!({
            "ok": true,
            "platform": demi_claude_code::platform::current().unwrap(),
            "installed": [{ "version": "2.1.278", "path": path }],
        })
    );
    assert_eq!(completion.exit_code, 0);

    // The program downloads over HTTPS only, from any address.
    let (document, completion) = invoke(
        &service,
        "claude-code.ensure",
        record("2.1.279", "http://127.0.0.1:9/claude"),
    )
    .await;
    let error = completion.error.unwrap();
    assert_eq!((completion.exit_code, error.code.as_str()), (1, "invalid_release"));
    assert_eq!(
        document,
        json!({ "ok": false, "code": "invalid_release", "message": error.message })
    );

    let (document, completion) = invoke(&service, "claude-code.ensure", b"{}".to_vec()).await;
    assert_eq!((document["ok"].clone(), document["code"].clone()), (json!(false), json!("invalid_release")));
    assert_eq!(completion.exit_code, 1);

    let (document, _) = invoke(&service, "claude-code.ensure", vec![b' '; 64 * 1024 + 1]).await;
    assert_eq!(document["code"], json!("invalid_release"));

    assert!(service.shutdown().await.unwrap().success());
}
