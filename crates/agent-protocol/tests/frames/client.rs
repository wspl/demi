//! What the browser sends: `fixtures/client-frames.json` holds every frame in
//! its wire shape.

use std::{
    future::Future,
    pin::pin,
    task::{Context, Poll, Waker},
};

use demi_agent_protocol::{ClientContent, ClientFrame, FrameError, decode_client_frame};
use demi_core::{DocumentSource, UserContentBlock};
use serde_json::{Value, json};

fn fixtures() -> Vec<Value> {
    serde_json::from_str(include_str!("fixtures/client-frames.json")).expect("the fixture is JSON")
}

fn fixture(kind: &str) -> Value {
    fixtures()
        .into_iter()
        .find(|fixture| fixture["type"] == kind)
        .unwrap_or_else(|| panic!("no {kind} frame"))
}

#[test]
fn every_client_frame_decodes_from_its_wire_shape() {
    let mut kinds = Vec::new();
    for fixture in fixtures() {
        let frame = decode_client_frame(&fixture.to_string())
            .unwrap_or_else(|error| panic!("{fixture}: {error}"));
        assert_eq!(serde_json::to_value(&frame).unwrap(), fixture);
        assert_eq!(frame.kind().to_string(), fixture["type"].as_str().unwrap());
        kinds.push(frame.kind());
    }
    kinds.dedup();
    assert_eq!(kinds.len(), 20, "one fixture of every frame");
}

/// `value` with the field at `pointer` set to `field`, adding it if absent.
fn with(mut value: Value, pointer: &str, field: Value) -> Value {
    let (parent, key) = pointer.rsplit_once('/').unwrap();
    match value.pointer_mut(parent).unwrap_or_else(|| panic!("{parent}")) {
        Value::Object(object) => {
            object.insert(key.to_owned(), field);
        }
        Value::Array(array) => array[key.parse::<usize>().unwrap()] = field,
        other => panic!("{pointer} is inside {other}"),
    }
    value
}

fn upload(file_name: &str) -> Value {
    with(fixture("send"), "/content/2/fileName", json!(file_name))
}

#[test]
fn client_frames_refuse_unknown_fields_nulls_and_values_outside_their_bounds() {
    let kept_attachment = json!({"type": "attachment", "path": "/home/demi/.demi/attachments/c1/a.txt"});
    let kept_image = json!({"type": "media", "media": {"type": "image", "ref": "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08", "mediaType": "image/png"}});
    let refused = [
        ("a session id and a working directory", with(with(fixture("open"), "/cwd", json!("/work")), "/sessionId", json!("s1"))),
        ("a provider instead of a model", json!({"type": "open", "provider": fixture("open")["model"]})),
        ("action metadata", with(fixture("send"), "/metadata", json!({}))),
        ("a field on a frame that has none", with(fixture("abort"), "/force", json!(true))),
        ("a frame type the protocol does not have", json!({"type": "shell_status", "commandId": "cmd-1"})),
        ("a frame without its message id", json!({"type": "send", "content": []})),
        ("an empty message id", with(fixture("send"), "/messageId", json!(""))),
        ("an unknown field in content", with(fixture("send"), "/content/0/lang", json!("en"))),
        ("inline media bytes", with(fixture("send"), "/content/0", json!({"type": "image", "source": {"type": "binary", "data": "iVBORw0KGgo=", "mediaType": "image/png"}}))),
        ("an upload without its reference", with(fixture("send"), "/content/2/ref", json!(""))),
        ("a file name with a separator", upload("reports/a.pdf")),
        ("a file name with a backslash", upload(r"reports\a.pdf")),
        ("a file name with NUL", upload("a\u{0}.pdf")),
        ("the file name .", upload(".")),
        ("the file name ..", upload("..")),
        ("a file name of 256 UTF-16 units", upload(&"😀".repeat(128))),
        ("a relative remote path", with(fixture("send"), "/content/3/path", json!("notes.md"))),
        ("a remote path with NUL", with(fixture("send"), "/content/3/path", json!("/a\u{0}b"))),
        ("a remote file without its device", with(fixture("send"), "/content/3/deviceId", json!(""))),
        ("an attachment reference in a send", with(fixture("send"), "/content/0", kept_attachment)),
        ("kept media in a steer", with(fixture("steer"), "/content/0", kept_image)),
        ("a switch timing that is null", with(fixture("set_provider"), "/apply", Value::Null)),
        ("a switch timing outside the set", with(fixture("set_provider"), "/apply", json!("later"))),
        ("an output limit of zero", with(fixture("open"), "/model/model/outputLimit", json!(0))),
        ("an edit without content", with(fixture("edit_and_send"), "/request/content", json!([]))),
        ("an edit of blank text only", with(fixture("edit_and_send"), "/request/content", json!([{"type": "text", "text": " \n\u{feff}"}]))),
        ("an edit against a version without an epoch", with(fixture("edit_and_send"), "/request/version/epoch", json!(""))),
        ("a negative revision", with(fixture("edit_and_send"), "/request/version/revision", json!(-1))),
        ("kept media whose reference is not a SHA-256", with(fixture("edit_and_send"), "/request/content/2/media/ref", json!("blob-1"))),
        ("a kept document without its name", with(fixture("edit_and_send"), "/request/content/4/media/fileName", json!(""))),
        ("an edit target that is empty", with(fixture("edit_and_send"), "/request/targetBlockId", json!(""))),
    ];
    for (why, frame) in refused {
        match decode_client_frame(&frame.to_string()) {
            Err(FrameError::Invalid(_)) => {}
            other => panic!("{why}: {other:?} for {frame}"),
        }
    }
}

#[test]
fn a_file_name_counts_utf16_units_and_may_start_with_dots() {
    for name in ["é".repeat(255), "😀".repeat(127), "...".into(), "..a".into(), ".env".into()] {
        let frame = upload(&name);
        assert!(decode_client_frame(&frame.to_string()).is_ok(), "{name}");
    }
}

#[test]
fn a_message_that_is_not_json_is_told_from_an_invalid_frame() {
    for text in ["not json", "{\"type\":", ""] {
        assert!(matches!(decode_client_frame(text), Err(FrameError::NotJson(_))), "{text:?}");
    }
    for text in ["[1, 2]", "{\"type\": \"nope\"}", "null"] {
        assert!(matches!(decode_client_frame(text), Err(FrameError::Invalid(_))), "{text:?}");
    }
    let error = decode_client_frame(&upload("..").to_string()).unwrap_err().to_string();
    assert!(error.contains("content[2].file_name"), "{error}");
    let error = decode_client_frame(&with(fixture("send"), "/content/0", json!({"type": "attachment", "path": "/a"})).to_string())
        .unwrap_err()
        .to_string();
    assert!(error.contains("content[0]"), "{error}");
}

/// The value of a future that never waits.
fn ready<T>(future: impl Future<Output = T>) -> T {
    match pin!(future).poll(&mut Context::from_waker(Waker::noop())) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("the future waited"),
    }
}

/// What a backend might make of the browser's content: the session's content
/// blocks, with an upload or a remote file standing for the text that names
/// it.
async fn resolve(content: Vec<ClientContent>) -> Result<Vec<UserContentBlock>, String> {
    let resolved = content
        .into_iter()
        .map(|part| match part {
            ClientContent::Text { text } => UserContentBlock::Text { text },
            ClientContent::Reference { reference } => UserContentBlock::Reference { reference },
            ClientContent::Upload { file_name, .. } => UserContentBlock::Text { text: format!("uploaded {file_name}") },
            ClientContent::RemoteFile { path, .. } => UserContentBlock::Reference { reference: path },
            ClientContent::Media { media } => media.into(),
            ClientContent::Attachment { path } => UserContentBlock::Text { text: format!("kept {path}") },
        })
        .collect();
    Ok(resolved)
}

#[test]
fn resolving_content_changes_only_the_frames_that_carry_it() {
    for fixture in fixtures() {
        let frame = decode_client_frame(&fixture.to_string()).unwrap();
        let kind = frame.kind();
        let resolved: ClientFrame<UserContentBlock> = ready(frame.map_content(resolve)).unwrap();
        assert_eq!(resolved.kind(), kind);
        let encoded = serde_json::to_value(&resolved).unwrap();
        if !matches!(fixture["type"].as_str().unwrap(), "send" | "steer" | "edit_and_send") {
            assert_eq!(encoded, fixture);
        }
    }

    let send = decode_client_frame(&fixture("send").to_string()).unwrap();
    let ClientFrame::Send { content, .. } = ready(send.map_content(resolve)).unwrap() else {
        unreachable!()
    };
    assert_eq!(content[2], UserContentBlock::Text { text: "uploaded report.pdf".into() });

    let edit = decode_client_frame(&fixture("edit_and_send").to_string()).unwrap();
    let ClientFrame::EditAndSend { request } = ready(edit.map_content(resolve)).unwrap() else {
        unreachable!()
    };
    assert_eq!(request.operation_id.as_str(), "edit-1");
    assert_eq!(request.version.revision, 12);
    let UserContentBlock::Document { source: DocumentSource::Ref { file_name, .. } } = &request.content[4] else {
        panic!("{:?}", request.content[4]);
    };
    assert_eq!(file_name, "spec.pdf");

    let failed = ready(
        decode_client_frame(&fixture("send").to_string())
            .unwrap()
            .map_content(|_| async { Err::<Vec<UserContentBlock>, _>("unavailable") }),
    );
    assert_eq!(failed.unwrap_err(), "unavailable");
}

#[test]
fn client_frames_are_strict_in_their_schema() {
    let schema = serde_json::to_value(schemars::schema_for!(ClientFrame)).unwrap();
    for variant in schema["oneOf"].as_array().unwrap() {
        assert_eq!(variant["additionalProperties"], false, "{}", variant["properties"]["type"]);
    }
}
