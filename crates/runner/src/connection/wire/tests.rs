use super::{Inbound, decode};

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
