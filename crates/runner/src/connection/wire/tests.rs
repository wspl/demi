use super::{Inbound, LogLinesLinesItem, Timestamp, decode, log_lines};

#[test]
fn decodes_binary_and_dates_written_by_the_typescript_codec() {
    match decode(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packages/runner-protocol/src/fixtures/binary.msgpack"
    )))
    .unwrap()
    {
        Inbound::JobStdin { job_id, bytes } => {
            assert_eq!(job_id, "job");
            assert_eq!(bytes.0, [0, 255, 13, 10]);
        }
        _ => panic!("wrong message type"),
    }
    match decode(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packages/runner-protocol/src/fixtures/time.msgpack"
    )))
    .unwrap()
    {
        Inbound::FsUtimes { atime, mtime, .. } => {
            assert_eq!(atime.0, -123456789);
            assert_eq!(mtime.0, 1700000000123);
        }
        _ => panic!("wrong message type"),
    }
    match decode(include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packages/runner-protocol/src/fixtures/job.msgpack"
    )))
    .unwrap()
    {
        Inbound::JobStart { context, .. } => {
            assert_eq!(context.conversation, "conversation");
            assert_eq!(context.caller.node(), Some("node"));
            assert_eq!(context.locale.time_zone, "Asia/Shanghai");
            assert_eq!(context.locale.languages, ["zh-CN", "en"]);
        }
        _ => panic!("wrong message type"),
    }
}

#[test]
fn rejects_array_disguised_as_binary_unknown_fields_and_trailing_data() {
    let invalid = serde_json::json!({"type":"job_stdin", "jobId":"bad", "bytes":[1,2]});
    assert!(decode(&rmp_serde::to_vec_named(&invalid).unwrap()).is_err());
    let invalid = serde_json::json!({"type":"ping", "extra":true});
    assert!(decode(&rmp_serde::to_vec_named(&invalid).unwrap()).is_err());
    let mut trailing = include_bytes!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../packages/runner-protocol/src/fixtures/job.msgpack"
    ))
    .to_vec();
    trailing.push(0);
    assert!(decode(&trailing).is_err());
}

#[test]
fn generated_wire_checks_optional_fields_and_nested_unions() {
    let valid = serde_json::json!({"type":"fs_stat", "id":"file", "path":"/work"});
    assert!(decode(&rmp_serde::to_vec_named(&valid).unwrap()).is_ok());
    let mut invalid = valid;
    invalid["cwd"] = serde_json::Value::Null;
    assert!(decode(&rmp_serde::to_vec_named(&invalid).unwrap()).is_err());
}

#[test]
fn log_read_bounds_its_limit_and_log_lines_carries_times_as_timestamps() {
    let valid = serde_json::json!({"type":"log_read", "id":"log", "limit":200});
    match decode(&rmp_serde::to_vec_named(&valid).unwrap()).unwrap() {
        Inbound::LogRead {
            id,
            since,
            limit,
            source,
        } => {
            assert_eq!(id, "log");
            assert_eq!(since, None);
            assert_eq!(limit, 200);
            assert_eq!(source, None);
        }
        _ => panic!("wrong message type"),
    }
    let valid = serde_json::json!({
        "type":"log_read", "id":"log", "since":41, "limit":1000, "source":"service:demi.builtin"
    });
    assert!(decode(&rmp_serde::to_vec_named(&valid).unwrap()).is_ok());
    for limit in [0, 1001] {
        let invalid = serde_json::json!({"type":"log_read", "id":"log", "limit":limit});
        assert!(decode(&rmp_serde::to_vec_named(&invalid).unwrap()).is_err());
    }

    let line = LogLinesLinesItem {
        at: Timestamp(1_700_000_000_123),
        source: "stream:browser.live".into(),
        conversation_id: Some("conversation".into()),
        text: "could not list tabs".into(),
    };
    let reply = log_lines("log".into(), vec![line.clone()], 42).unwrap();
    #[derive(serde::Deserialize)]
    struct Reply {
        r#type: String,
        id: String,
        lines: Vec<LogLinesLinesItem>,
        next: u64,
    }
    let reply: Reply = rmp_serde::from_slice(&reply.into_bytes()).unwrap();
    assert_eq!(reply.r#type, "log_lines");
    assert_eq!(reply.id, "log");
    assert_eq!(reply.lines, [line]);
    assert_eq!(reply.next, 42);
    let empty_source = LogLinesLinesItem {
        source: String::new(),
        ..reply.lines[0].clone()
    };
    assert!(log_lines("log".into(), vec![empty_source], 42).is_err());
}
