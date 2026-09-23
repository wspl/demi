use bytes::Bytes;
use demi_core::{CommandId, ShellId, StreamKind};
use demi_shell::{CommandRecord, CommandState, Ending, TAIL_CHARS, final_stdout_boundary};

fn new_record() -> CommandRecord {
    CommandRecord::new(
        ShellId::try_from("shell").unwrap(),
        CommandId::try_from("command").unwrap(),
    )
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn views_deliver_each_stream_once_within_the_budget_between_characters() {
    let mut record = new_record();
    record.append_output(StreamKind::Stdout, "héllo ");
    record.append_output(StreamKind::Stderr, "warn\n");
    record.append_output(StreamKind::Stdout, "wörld\n");
    tokio::time::advance(std::time::Duration::from_millis(250)).await;

    // "hé" is three bytes: a two-byte budget ends before the é, never inside it.
    let first = record.status(2, None);
    assert_eq!(first.stdout.delta, "h");
    assert_eq!(first.stdout.offset, 1);
    assert!(first.stdout.truncated);
    assert_eq!(first.stderr.delta, "wa");
    assert_eq!(first.output.text, "h");
    assert_eq!(first.running_ms, 250);
    assert!(matches!(first.state, CommandState::Running { hint: None }));

    let rest = record.status(0, Some("Working".into()));
    assert_eq!(rest.stdout.delta, "éllo wörld\n");
    assert_eq!(rest.stdout.bytes, "héllo wörld\n".len() as u64);
    assert!(!rest.stdout.truncated);
    assert_eq!(rest.stdout.tail, "héllo wörld\n");
    assert_eq!(rest.stderr.delta, "rn\n");
    let chunks: Vec<_> = rest
        .output
        .chunks
        .iter()
        .map(|chunk| (chunk.stream, chunk.text.as_str()))
        .collect();
    assert_eq!(
        chunks,
        [
            (StreamKind::Stdout, "éllo "),
            (StreamKind::Stderr, "warn\n"),
            (StreamKind::Stdout, "wörld\n"),
        ]
    );
    assert!(
        matches!(rest.state, CommandState::Running { hint: Some(ref hint) } if hint == "Working")
    );

    let empty = record.status(0, None);
    assert_eq!(empty.stdout.delta, "");
    assert_eq!(empty.output.chunks, []);
    assert_eq!(empty.idle_ms, 250);
}

#[tokio::test(flavor = "current_thread")]
async fn an_exit_appends_what_it_adds_and_a_binary_stdout_is_presented_anew() {
    let mut record = new_record();
    record.append_output(StreamKind::Stdout, "head\n");
    let seen = record.status(0, None);
    assert_eq!(seen.output.text, "head\n");
    record.settle(
        Ending::Exited(3),
        "head\n[gap]\ntail\n".into(),
        "oops\n".into(),
        None,
    );
    let exited = record.status(0, None);
    assert_eq!(exited.output.text, "[gap]\ntail\noops\n");
    assert_eq!(exited.stdout.delta, "[gap]\ntail\n");
    assert!(matches!(
        exited.state,
        CommandState::Exited {
            exit_code: 3,
            binary_stdout: None
        }
    ));

    let mut binary = new_record();
    binary.append_output(StreamKind::Stdout, "\u{fffd}PNG");
    binary.status(0, None);
    let (text, bytes) = final_stdout_boundary(
        Bytes::from_static(b"\x89PNG\r\n"),
        4,
        Some("/out/stdout.txt"),
    );
    assert_eq!(
        text,
        "<binary stdout: 6 bytes, exceeds the 4-byte binary limit; raw bytes at /out/stdout.txt>\n"
    );
    binary.settle(Ending::Exited(0), text.clone(), String::new(), bytes);
    let status = binary.status(0, None);
    assert_eq!(status.stdout.delta, text);
    assert_eq!(status.output.text, text);
    let CommandState::Exited {
        binary_stdout: Some(stdout),
        ..
    } = status.state
    else {
        panic!("a binary stdout")
    };
    assert_eq!(&stdout.bytes[..], b"\x89PNG");
    assert!(stdout.info.truncated);
    assert_eq!((stdout.info.total_bytes, stdout.info.limit_bytes), (6, 4));

    let (text, bytes) = final_stdout_boundary(Bytes::from_static(b"\xff"), 16, None);
    assert_eq!(
        text,
        "<binary stdout: 1 bytes; not kept beyond this view>\n"
    );
    assert!(!bytes.unwrap().info.truncated);
    assert_eq!(
        final_stdout_boundary(Bytes::from_static(b"text"), 16, None),
        ("text".into(), None)
    );
}

#[test]
fn a_stop_after_the_streams_end_is_aborted_and_the_merged_view_follows_the_streams() {
    let mut record = new_record();
    record.append_output(StreamKind::Stderr, "partial");
    // An error the environment adds to stderr, not a continuation of it,
    // rebuilds the merged view from the streams.
    record.settle(Ending::Aborted, String::new(), "different\n".into(), None);
    let status = record.status(0, None);
    assert!(matches!(status.state, CommandState::Aborted));
    assert_eq!(status.output.text, "different\n");
    assert_eq!(status.output.offset, "different\n".len() as u64);

    let long = "x".repeat(TAIL_CHARS) + "é";
    let mut record = new_record();
    record.append_output(StreamKind::Stdout, &long);
    record.mark_aborted();
    let status = record.status(1, None);
    assert!(matches!(status.state, CommandState::Aborted));
    assert_eq!(status.stdout.tail.chars().count(), TAIL_CHARS);
    assert!(status.stdout.tail.ends_with('é'));
    assert_eq!(status.output.tail.chars().count(), TAIL_CHARS);
}
