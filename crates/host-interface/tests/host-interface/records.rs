use std::sync::Arc;

use bytes::Bytes;
use demi_host_interface::{
    BinaryOutput, CommandRecord, CommandState, Ending, OutputRecord, PageState, Seen, TAIL_CHARS,
    WholeOutput,
};
use demi_shared_types::{BinaryStdout, CommandId, StreamKind};

fn new_record() -> CommandRecord {
    CommandRecord::new(
        CommandId::try_from("command").unwrap(),
        "call".into(),
    )
}

#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn byte_views_deliver_each_stream_once_and_model_looks_repeat_unfinished_lines() {
    let mut record = new_record();
    record.append_output(StreamKind::Stdout, "héllo ");
    record.append_output(StreamKind::Stderr, "warn\n");
    record.append_output(StreamKind::Stdout, "wörld\n");
    tokio::time::advance(std::time::Duration::from_millis(250)).await;

    // "hé" is three bytes: a two-byte budget ends before the é, never inside it.
    let first = record.status(2, None, Vec::new());
    assert_eq!(first.stdout.delta, "h");
    assert_eq!(first.stdout.offset, 1);
    assert!(first.stdout.truncated);
    assert_eq!(first.stderr.delta, "wa");
    assert_eq!(first.output.text, "h");
    assert_eq!(first.output.offset, 0);
    assert_eq!(first.running_ms, 250);
    assert!(matches!(first.state, CommandState::Running { hint: None, .. }));

    let rest = record.status(0, Some("Working".into()), Vec::new());
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
            (StreamKind::Stdout, "héllo "),
            (StreamKind::Stderr, "warn\n"),
            (StreamKind::Stdout, "wörld\n"),
        ]
    );
    assert!(
        matches!(rest.state, CommandState::Running { hint: Some(ref hint), .. } if hint == "Working")
    );

    let empty = record.status(0, None, Vec::new());
    assert_eq!(empty.stdout.delta, "");
    assert_eq!(empty.output.chunks, []);
    assert_eq!(empty.idle_ms, 250);

    // A view names the line of the merged output its text starts in.
    record.append_output(StreamKind::Stdout, "next\n");
    let more = record.status(0, None, Vec::new());
    assert_eq!((more.output.text.as_str(), more.output.line), ("next\n", 3));
}

/// Once the command ended, the model's next look gives its whole output,
/// with how much of each stream the model had seen, and every look after it
/// gives nothing new.
#[test]
fn the_end_gives_the_whole_output_once_with_what_the_model_had_seen() {
    let mut record = new_record();
    record.append_output(StreamKind::Stdout, "head\n");
    assert_eq!(record.status(0, None, Vec::new()).output.text, "head\n");
    let whole = Arc::new(WholeOutput::new(
        vec![
            OutputRecord::Output(StreamKind::Stdout, Bytes::from_static(b"head\nmore\n")),
            OutputRecord::Output(StreamKind::Stderr, Bytes::from_static(b"oops\n")),
        ],
        None,
    ));
    let binary = BinaryOutput {
        bytes: Bytes::from_static(b"\x89PNG"),
        info: BinaryStdout {
            truncated: false,
            total_bytes: 4,
            limit_bytes: 16,
        },
    };
    assert!(record.settle(
        Ending::Exited(3),
        whole.clone(),
        Some(binary.clone()),
        Vec::new(),
        ""
    ));
    let exited = record.status(0, None, Vec::new());
    let view = exited.whole.expect("the whole output");
    assert_eq!(view.output, whole);
    assert_eq!(
        view.seen,
        Seen {
            stdout: 5,
            stderr: 0
        }
    );
    assert_eq!(
        exited.state,
        CommandState::Exited {
            exit_code: 3,
            binary_stdout: Some(binary),
            media: Vec::new(),
        }
    );
    let again = record.status(0, None, Vec::new()).whole.expect("the whole output");
    assert_eq!(
        again.seen,
        Seen {
            stdout: u64::MAX,
            stderr: u64::MAX
        }
    );
}

#[test]
fn a_stop_ends_the_command_aborted_and_one_whose_streams_never_ended_keeps_its_views() {
    let mut record = new_record();
    record.append_output(StreamKind::Stderr, "partial");
    assert!(record.settle(
        Ending::Aborted,
        Arc::new(WholeOutput::default()),
        None,
        Vec::new(),
        ""
    ));
    assert!(matches!(
        record.status(0, None, Vec::new()).state,
        CommandState::Aborted
    ));

    let long = "x".repeat(TAIL_CHARS) + "é";
    let mut record = new_record();
    record.append_output(StreamKind::Stdout, &long);
    record.mark_aborted();
    let status = record.status(1, None, Vec::new());
    assert!(matches!(status.state, CommandState::Aborted));
    assert!(status.whole.is_none());
    assert_eq!(status.stdout.tail.chars().count(), TAIL_CHARS);
    assert!(status.stdout.tail.ends_with('é'));
    assert_eq!(status.output.tail.chars().count(), TAIL_CHARS);
}

/// The pages' view keeps the last characters of the output in the order it
/// came and how many there were, whatever the model read, and ends with the
/// command (`runtime.md` § Live output).
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn the_pages_view_keeps_the_newest_characters_and_their_count_until_the_end() {
    let mut record = new_record();
    assert!(record.append_output(StreamKind::Stdout, "é"));
    assert!(!record.append_output(StreamKind::Stdout, ""));
    // The model's read moves only the model's place.
    record.status(0, None, Vec::new());
    let beyond = "x".repeat(TAIL_CHARS);
    assert!(record.append_page_output(&beyond));
    tokio::time::advance(std::time::Duration::from_millis(40)).await;
    let view = record.page_view();
    assert_eq!(view.tail, beyond);
    assert_eq!(view.chars, 1 + TAIL_CHARS as u64);
    assert_eq!(
        (view.state, view.running_ms, view.tool_use_id.as_str()),
        (PageState::Running, 40, "call")
    );
    // Output only the pages' view holds is not the model's. The model still
    // sees its unfinished line from the previous look.
    assert_eq!(record.status(0, None, Vec::new()).output.text, "é");

    // The end adds what it brings, once; nothing changes the view after it.
    assert!(record.settle(
        Ending::Exited(2),
        Arc::new(WholeOutput::default()),
        None,
        Vec::new(),
        "end\n"
    ));
    assert!(!record.append_output(StreamKind::Stderr, "late"));
    assert!(!record.mark_aborted());
    let view = record.page_view();
    assert_eq!(view.state, PageState::Exited { exit_code: 2 });
    assert!(
        view.tail.ends_with("xend\n"),
        "{:?}",
        &view.tail[view.tail.len() - 8..]
    );
    assert_eq!(view.tail.chars().count(), TAIL_CHARS);
    assert_eq!(view.chars, 1 + TAIL_CHARS as u64 + 4);
}
