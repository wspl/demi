use demi_runner_process::lines::{LINE_BYTES, LineSplitter};

#[test]
fn splits_chunks_that_do_not_end_at_a_newline() {
    let mut splitter = LineSplitter::default();
    assert_eq!(splitter.push(b"first li"), Vec::<String>::new());
    assert_eq!(
        splitter.push(b"ne\r\nsecond\n\nthi"),
        ["first line", "second"]
    );
    assert_eq!(splitter.push(b"rd"), Vec::<String>::new());
    assert_eq!(splitter.finish().as_deref(), Some("third"));

    let mut splitter = LineSplitter::default();
    let long = vec![b'y'; LINE_BYTES + 3];
    let lines = splitter.push(&long);
    assert_eq!(lines.len(), 1);
    assert_eq!(lines[0].len(), LINE_BYTES);
    assert_eq!(splitter.finish().as_deref(), Some("yyy"));
    assert_eq!(LineSplitter::default().finish(), None);
}
