//! The builtin package's contract: what each boundary accepts and refuses,
//! and the JSON its results and messages make.

use demi_builtin_protocol::{
    DecodeError, Operation, OperationError,
    browser::{
        ActionResult, AssetsExportResult, BrowserErrorCode, BrowserFailure, BrowserInput,
        BrowserOperation, BrowserQuery, BrowserTarget, ErrorDetails, ExportedAsset, Load,
        MouseButton, NodeRef, OPERATIONS, ReadResult, TabId,
    },
    capture::{CaptureEvent, FrameHeader},
    file::FileOperation,
    live::{FileHeader, LiveModuleMessage, LiveViewerMessage, VideoHeader},
    release::BrowserRelease,
};
use serde_json::{Value, json};

const TAB: &str = "t_AAAAAAAAAAAAAAAAAAAAAA";
const REF: &str = "e_BBBBBBBBBBBBBBBBBBBBBB";

fn parse(operation: &str, args: Value) -> Result<BrowserOperation, DecodeError> {
    BrowserOperation::parse(operation, args)
}

#[test]
fn every_operation_decodes_its_smallest_input() {
    let smallest = |operation: &str| -> Value {
        match operation {
            "open" => json!({"url": "about:blank"}),
            "tabs" => json!({}),
            "content.fetch" => json!({"url": ["https://example.test/"]}),
            "goto" => json!({"tab": TAB, "url": "about:blank"}),
            "probe" => json!({"tab": TAB, "xy": "1,2"}),
            "drag" => json!({"tab": TAB, "point": ["1,2", "3,4"]}),
            "fill" | "type" | "select-text" => json!({"tab": TAB, "text": "hello"}),
            "key" => json!({"tab": TAB, "key": "Enter"}),
            "check" => json!({"tab": TAB, "value": true}),
            "upload" => json!({"tab": TAB, "file": ["a.txt"]}),
            "eval" => json!({"tab": TAB, "expression": "1"}),
            "viewport.set" => json!({"tab": TAB, "width": 800, "height": 600}),
            "cdp.send" => json!({"tab": TAB, "method": "Page.enable", "params": "{}"}),
            "assets.export" => json!({"tab": TAB, "inventory": "i", "output-dir": "out"}),
            "webmcp.call" => json!({"tab": TAB, "tool": "t", "tools": "g", "arguments": "{}"}),
            _ => json!({"tab": TAB}),
        }
    };
    assert_eq!(OPERATIONS.len(), 47);
    for operation in OPERATIONS {
        let name = operation.strip_prefix("browser.").unwrap();
        let decoded = parse(name, smallest(name)).unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(decoded.name(), name);
        let expects_tab = !matches!(name, "open" | "tabs" | "content.fetch");
        assert_eq!(decoded.tab().is_some(), expects_tab, "{name}");
    }
    assert!(matches!(parse("unknown", json!({})), Err(DecodeError::UnknownOperation(_))));
}

#[test]
fn inputs_refuse_unknown_fields_nulls_and_values_outside_their_bounds() {
    let invalid = [
        ("info", json!({"tab": TAB, "extra": 1})),
        ("info", json!({"tab": TAB, "timeout": null})),
        ("info", json!({"tab": TAB, "timeout": 0})),
        ("info", json!({"tab": TAB, "timeout": 300_001})),
        ("info", json!({"tab": "t_short"})),
        ("click", json!({"tab": TAB, "ref": "t_AAAAAAAAAAAAAAAAAAAAAA"})),
        ("click", json!({"tab": TAB, "count": 3})),
        ("click", json!({"tab": TAB, "button": "back"})),
        ("click", json!({"tab": TAB, "modifier": ["Hyper"]})),
        ("click", json!({"tab": TAB, "role": ""})),
        ("click", json!({"tab": TAB, "nth": 1000})),
        ("tabs", json!({"limit": 0})),
        ("tabs", json!({"limit": 1001})),
        ("goto", json!({"tab": TAB, "url": "about:blank", "load": "idle"})),
        ("drag", json!({"tab": TAB, "point": ["1,2"]})),
        ("content.fetch", json!({"url": []})),
        ("content.fetch", json!({"url": vec!["https://example.test/"; 11]})),
        ("viewport.set", json!({"tab": TAB, "width": 800, "height": 600, "scale": 5})),
        ("upload", json!({"tab": TAB, "file": [""]})),
        ("read", json!({"tab": TAB, "property": "outer-html"})),
        ("type", json!({"tab": TAB, "text": "x".repeat(1024 * 1024 + 1)})),
    ];
    for (operation, args) in invalid {
        assert!(parse(operation, args.clone()).is_err(), "{operation} {args}");
    }
    // A limit counts Unicode scalar values, as the page's schemas and JSON
    // Schema do: 😀 is one, though two UTF-16 units.
    let emoji = "😀".repeat(4097);
    assert!(parse("click", json!({"tab": TAB, "css": emoji})).is_err());
    assert!(parse("click", json!({"tab": TAB, "css": "😀".repeat(4096)})).is_ok());
}

#[test]
fn inputs_answer_their_tab_target_wait_and_deadline() {
    let click = parse(
        "click",
        json!({
            "tab": TAB, "role": "button", "name-pattern": "^Save", "frame": [REF], "nth": 2,
            "button": "right", "wait-url": "**/done", "timeout": 5000,
        }),
    )
    .unwrap();
    assert_eq!(click.tab().map(TabId::as_str), Some(TAB));
    assert_eq!(click.wait_url(), Some("**/done"));
    assert_eq!(click.timeout(), std::time::Duration::from_millis(5000));
    let target = click.target().unwrap();
    assert_eq!(target.role.as_deref(), Some("button"));
    assert_eq!(target.name_pattern.as_deref(), Some("^Save"));
    assert_eq!(target.frame.as_deref(), Some(&[REF.parse::<NodeRef>().unwrap()][..]));
    assert_eq!(target.nth, Some(2));
    let BrowserOperation::Click(input) = click else { panic!("not a click") };
    assert_eq!(input.button, Some(MouseButton::Right));
    // `open` may take a cold start; everything else has thirty seconds.
    let open = parse("open", json!({"url": "about:blank"})).unwrap();
    assert_eq!(open.timeout(), std::time::Duration::from_secs(300));
    let BrowserOperation::Open(input) = open else { panic!("not an open") };
    assert_eq!(input.load.unwrap_or_default(), Load::DomContentLoaded);
    assert_eq!(input.timeout(), std::time::Duration::from_secs(300));
    let info = parse("info", json!({"tab": TAB})).unwrap();
    assert_eq!(info.timeout(), std::time::Duration::from_secs(30));
    assert_eq!(info.target(), None);
}

#[test]
fn a_query_tree_has_one_base_in_every_branch() {
    let query = BrowserQuery::parse(
        r#"{"and": [{"match": {"role": "row"}}, {"match": {"text-match": "Order A"}}],
            "has": {"match": {"role": "button", "name": "Delete"}}, "nth": 0}"#,
    )
    .unwrap();
    let branch = &query.and.as_ref().unwrap()[1];
    let target = BrowserTarget::from(branch.r#match.clone().unwrap());
    assert_eq!(target.text_match.as_deref(), Some("Order A"));
    for invalid in [
        r#"{"match": {"role": "row"}, "or": [{"match": {"role": "cell"}}]}"#,
        r#"{"has": {"match": {"role": "row"}}}"#,
        r#"{"match": {"role": "row"}, "has": {"hasText": "x"}}"#,
        r#"{"and": []}"#,
        r#"{"match": {"role": "row", "nth": 1}}"#,
        r#"{"match": {"role": "row"}, "visible": null}"#,
    ] {
        assert!(BrowserQuery::parse(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn file_arguments_refuse_empty_old_text_and_zero_positions() {
    let edit = |args: Value| FileOperation::parse("file.edit", args);
    assert!(edit(json!({"path": "a", "old": "x", "new": "y", "occurrence": 2})).is_ok());
    for invalid in [
        json!({"path": "a", "old": "", "new": "y"}),
        json!({"path": "a", "old": "x", "new": "y", "occurrence": 0}),
        json!({"path": "a", "old": "x", "new": "y", "context": 0}),
        json!({"path": "a", "old": "x", "new": "y", "context": null}),
        json!({"path": "a", "old": "x"}),
    ] {
        assert!(edit(invalid.clone()).is_err(), "{invalid}");
    }
    assert!(FileOperation::parse("file.read", json!({"path": "a", "extra": true})).is_err());
    assert!(FileOperation::parse("file.remove", json!({"path": "a"})).is_err());
    assert_eq!(Operation::names().count(), 4 + 47 + 1);
}

#[test]
fn an_invocation_decodes_to_the_part_of_the_package_its_name_names() {
    assert!(matches!(
        Operation::parse("file.read", json!({"path": "a"})),
        Ok(Operation::File(FileOperation::Read(_)))
    ));
    assert!(matches!(
        Operation::parse("browser.tabs", json!({})),
        Ok(Operation::Browser(_))
    ));
    assert!(matches!(
        Operation::parse("browser.live", json!({})),
        Ok(Operation::Live)
    ));
    // Each part reports a refused argument itself.
    assert!(matches!(
        Operation::parse("file.read", json!({})),
        Err(OperationError::File(_))
    ));
    assert!(matches!(
        Operation::parse("browser.nothing", json!({})),
        Err(OperationError::Browser(_))
    ));
    assert!(matches!(
        Operation::parse("browser.live", json!({"tab": "t"})),
        Err(OperationError::Browser(_))
    ));
    assert!(matches!(
        Operation::parse("claude.ensure", json!({})),
        Err(OperationError::Unknown(name)) if name == "claude.ensure"
    ));
    // Every listed name decodes, so the descriptor lists nothing unserved.
    for name in Operation::names() {
        assert!(
            !matches!(Operation::parse(name, json!({})), Err(OperationError::Unknown(_))),
            "{name}"
        );
    }
}

#[test]
fn results_and_failures_print_the_documented_names() {
    let action = ActionResult {
        operation: "click".into(),
        target: None,
        result: json!("completed"),
        url: Some("https://example.test/".into()),
        opened_tabs: Some(vec![TAB.parse().unwrap()]),
        dialog: None,
    };
    assert_eq!(
        serde_json::to_value(&action).unwrap(),
        json!({"operation": "click", "result": "completed", "url": "https://example.test/",
            "openedTabs": [TAB]}),
    );
    let read: ReadResult = serde_json::from_value(json!({"values": [1, "a"], "truncated": false})).unwrap();
    assert!(matches!(read, ReadResult::All { .. }));
    assert!(serde_json::from_value::<ReadResult>(json!({"value": 1, "extra": 2})).is_err());
    let export = AssetsExportResult {
        directory: "/out".into(),
        manifest: "/out/manifest.json".into(),
        files: vec![ExportedAsset {
            id: "a".into(),
            path: "/out/a.png".into(),
            bytes: 3,
            mime_type: "image/png".into(),
        }],
    };
    let failure = BrowserFailure {
        code: BrowserErrorCode::PartialFailure,
        message: "some browser items failed".into(),
        details: Some(ErrorDetails {
            action: Some(demi_builtin_protocol::browser::ActionProgress::NotStarted),
            tab: Some(TAB.into()),
            debugging_callers: Some(vec!["node".into()]),
            export: Some(export),
            ..ErrorDetails::default()
        }),
    };
    let printed = serde_json::to_value(&failure).unwrap();
    assert_eq!(
        printed,
        json!({"code": "partial_failure", "message": "some browser items failed", "details": {
            "action": "not_started", "tab": TAB, "debuggingCallers": ["node"],
            "directory": "/out", "manifest": "/out/manifest.json",
            "files": [{"id": "a", "path": "/out/a.png", "bytes": 3, "mimeType": "image/png"}],
        }}),
    );
    assert_eq!(serde_json::from_value::<BrowserFailure>(printed).unwrap(), failure);
    assert_eq!(BrowserErrorCode::OutcomeUnknown.to_string(), "outcome_unknown");
}

#[test]
fn live_messages_decode_as_the_page_sends_them() {
    for message in [
        json!({"type": "hello", "platform": "mac"}),
        json!({"type": "panel", "width": 800, "height": 600, "devicePixelRatio": 2,
            "screenWidth": 1512, "screenHeight": 982}),
        json!({"type": "watch", "tab": null}),
        json!({"type": "mode", "tab": TAB, "mode": "mobile"}),
        json!({"type": "pointer", "tab": TAB, "action": "down", "x": 10.5, "y": 0, "button": "left",
            "buttons": 1, "clickCount": 1, "modifiers": 0}),
        json!({"type": "key", "tab": TAB, "action": "down", "key": "a", "code": "KeyA",
            "keyCode": 65, "modifiers": 8, "repeat": false, "location": 0, "text": "A",
            "altGraph": false}),
        json!({"type": "upload", "tab": TAB, "token": "0f8e8b2c-1d2a-4c3b-9a8f-7e6d5c4b3a21",
            "revision": 3, "upload": 1, "files": [{"name": "a.txt", "mimeType": "text/plain", "size": 3}]}),
        json!({"type": "ack", "generation": 1, "sequence": 7, "decodeQueue": 0}),
        json!({"type": "release"}),
    ] {
        let bytes = serde_json::to_vec(&message).unwrap();
        LiveViewerMessage::decode(&bytes).unwrap_or_else(|error| panic!("{message}: {error}"));
    }
    for message in [
        json!({"type": "watch"}),
        json!({"type": "mode", "tab": TAB, "mode": "custom"}),
        json!({"type": "pointer", "tab": TAB, "action": "down", "x": 5000, "y": 0, "button": "left",
            "buttons": 1, "clickCount": 1, "modifiers": 0}),
        json!({"type": "upload", "tab": TAB, "token": "not-a-token", "revision": 3, "upload": 1,
            "files": []}),
        json!({"type": "upload", "tab": TAB, "token": "0f8e8b2c-1d2a-4c3b-9a8f-7e6d5c4b3a21",
            "revision": 3, "upload": 1, "files": [{"name": "../a", "mimeType": "", "size": 3}]}),
        json!({"type": "release", "extra": true}),
        json!({"type": "unknown"}),
    ] {
        let bytes = serde_json::to_vec(&message).unwrap();
        assert!(LiveViewerMessage::decode(&bytes).is_err(), "{message}");
    }
    let ended = serde_json::to_value(LiveModuleMessage::Ended {
        reason: demi_builtin_protocol::live::EndReason::BrowserEnded,
    })
    .unwrap();
    assert_eq!(ended, json!({"type": "ended", "reason": "browser_ended"}));
    let state = serde_json::to_value(LiveModuleMessage::State {
        running: false,
        tabs: Vec::new(),
        watched: None,
    })
    .unwrap();
    assert_eq!(state, json!({"type": "state", "running": false, "tabs": [], "watched": null}));
}

#[test]
fn frame_headers_have_their_documented_layout() {
    let header = VideoHeader {
        tab: TAB.parse().unwrap(),
        generation: 2,
        sequence: 9,
        key: true,
        timestamp: 1.5,
        width: 1280,
        height: 720,
    };
    let mut bytes = Vec::new();
    header.write(&mut bytes);
    assert_eq!(bytes.len(), VideoHeader::BYTES);
    assert_eq!(&bytes[..24], TAB.as_bytes());
    assert_eq!(&bytes[24..33], [0, 0, 0, 2, 0, 0, 0, 9, 1]);
    assert_eq!(&bytes[44..], [5, 0, 2, 208]);
    let (file, data) = FileHeader::split(&[0, 0, 0, 7, 0, 0, 0, 1, b'a']).unwrap();
    assert_eq!((file.upload, file.file, data), (7, 1, &b"a"[..]));
    assert!(FileHeader::split(&[0; 7]).is_err());
    let mut capture = vec![0, 0, 0, 3, 0, 0, 0, 4, 1, 0, 0, 0];
    capture.extend_from_slice(&2.5_f64.to_be_bytes());
    capture.extend_from_slice(&[5, 0, 2, 208]);
    capture.extend_from_slice(&[0; 8]);
    capture.extend_from_slice(&[0, 0, 0, 1]);
    let (frame, data) = FrameHeader::split(&capture).unwrap();
    assert_eq!((frame.capture, frame.sequence, frame.key), (3, 4, true));
    assert_eq!((frame.timestamp, frame.width, frame.height), (2.5, 1280, 720));
    assert_eq!(data, [0, 0, 0, 1]);
    assert!(FrameHeader::split(&capture[..31]).is_err());
}

#[test]
fn capture_events_decode_and_unknown_events_are_refused() {
    assert_eq!(CaptureEvent::decode(r#"{"type":"ready"}"#).unwrap(), CaptureEvent::Ready {});
    assert_eq!(
        CaptureEvent::decode(r#"{"type":"error","capture":2,"message":"no track"}"#).unwrap(),
        CaptureEvent::Error { capture: 2, message: "no track".into() },
    );
    for invalid in [
        r#"{"type":"paused","capture":1}"#,
        r#"{"type":"started"}"#,
        r#"{"type":"started","capture":1,"extra":true}"#,
        "not json",
    ] {
        assert!(CaptureEvent::decode(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn release_records_are_checked() {
    let release = BrowserRelease::pinned().unwrap();
    assert!(release.platforms.iter().any(|platform| platform.target == "aarch64-apple-darwin"));
    let mut record = serde_json::to_value(&release).unwrap();
    record["version"] = json!("153.0.8010");
    assert!(BrowserRelease::parse(&record.to_string()).is_err());
    record["version"] = json!("153.0.8010.36");
    record["platforms"][0]["sha256"] = json!("F".repeat(64));
    assert!(BrowserRelease::parse(&record.to_string()).is_err());
    record["platforms"][0]["sha256"] = json!("f".repeat(64));
    record["platforms"][0]["url"] = json!("not a url");
    assert!(BrowserRelease::parse(&record.to_string()).is_err());
}

#[test]
fn handles_are_checked_and_made_from_random_bytes() {
    let tab = TabId::from_random([7; 16]);
    assert_eq!(tab.as_str().len(), 24);
    assert_eq!(tab.as_str().parse::<TabId>().unwrap(), tab);
    for invalid in ["t_", "e_AAAAAAAAAAAAAAAAAAAAAA", "t_AAAAAAAAAAAAAAAAAAAAA!", "t_AAAAAAAAAAAAAAAAAAAAAAA"] {
        assert!(invalid.parse::<TabId>().is_err(), "{invalid}");
    }
    assert_eq!(serde_json::to_value(&tab).unwrap(), json!(tab.as_str()));
}
