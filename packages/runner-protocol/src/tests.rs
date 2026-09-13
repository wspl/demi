use crate::{Inbound, decode};

#[test]
fn decodes_binary_and_dates_written_by_the_typescript_codec() {
    match decode(include_bytes!("fixtures/binary.msgpack")).unwrap() {
        Inbound::FsWriteFile {
            data,
            create_parents,
            ..
        } => {
            assert_eq!(data.0, [0, 255, 13, 10]);
            assert_eq!(create_parents, Some(true));
        }
        _ => panic!("wrong message type"),
    }
    match decode(include_bytes!("fixtures/time.msgpack")).unwrap() {
        Inbound::FsUtimes { atime, mtime, .. } => {
            assert_eq!(atime.0, -123456789);
            assert_eq!(mtime.0, 1700000000123);
        }
        _ => panic!("wrong message type"),
    }
    assert!(matches!(
        decode(include_bytes!("fixtures/job.msgpack")).unwrap(),
        Inbound::JobStart { .. }
    ));
}

#[test]
fn rejects_array_disguised_as_binary_unknown_fields_and_trailing_data() {
    let invalid =
        serde_json::json!({"type":"fs_writeFile", "id":"bad", "path":"/tmp/data", "data":[1,2]});
    assert!(decode(&rmp_serde::to_vec_named(&invalid).unwrap()).is_err());
    let invalid = serde_json::json!({"type":"ping", "extra":true});
    assert!(decode(&rmp_serde::to_vec_named(&invalid).unwrap()).is_err());
    let mut trailing = include_bytes!("fixtures/job.msgpack").to_vec();
    trailing.push(0);
    assert!(decode(&trailing).is_err());
}
