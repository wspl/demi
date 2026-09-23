use std::{cell::RefCell, convert::Infallible, rc::Rc, time::Duration};

use bytes::Bytes;
use demi_host_remote::{PipeError, PipeRefusal, Pipes};
use futures_util::{StreamExt, stream};
use tokio::sync::mpsc;

fn chunks(
    parts: &[&'static str],
) -> impl futures_util::Stream<Item = Result<Bytes, Infallible>> + use<> {
    stream::iter(
        parts
            .iter()
            .map(|part| Ok(Bytes::from_static(part.as_bytes())))
            .collect::<Vec<_>>(),
    )
}

/// A body whose chunks the test sends when it likes.
fn body() -> (
    mpsc::Sender<Result<Bytes, Infallible>>,
    impl futures_util::Stream<Item = Result<Bytes, Infallible>>,
) {
    let (sender, receiver) = mpsc::channel(1);
    let body = stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|chunk| (chunk, receiver))
    });
    (sender, body)
}

async fn collect(
    mut stream: futures_util::stream::BoxStream<
        'static,
        Result<Bytes, demi_host_remote::PipeFailure>,
    >,
) -> Vec<u8> {
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        bytes.extend_from_slice(&chunk.unwrap());
    }
    bytes
}

#[tokio::test(flavor = "local")]
async fn a_device_put_streams_into_a_device_get_and_answers_once_drained() {
    let pipes = Pipes::new(Duration::from_secs(5));
    // The sink is named after the pipe is minted.
    let pipe = pipes.mint(Some("a"), None);
    pipe.sink_to("b").unwrap();
    assert_eq!(pipe.sink_to("a"), Err(PipeError::AlreadyFixed("sink")));
    let payload: Vec<u8> = (0..2 * 1024 * 1024)
        .map(|index| (index & 0xff) as u8)
        .collect();
    let parts: Vec<_> = payload
        .chunks(64 * 1024)
        .map(|part| Ok::<_, Infallible>(Bytes::copy_from_slice(part)))
        .collect();
    let mut sink = pipes.claim_sink(pipe.id(), "b").unwrap();
    let source = pipes.claim_source(pipe.id(), "a").unwrap();
    let put = tokio::task::spawn_local(source.pump(stream::iter(parts)));
    sink.source_arrived().await.unwrap();
    assert_eq!(collect(sink.into_stream()).await, payload);
    put.await.unwrap().unwrap();
    pipe.done().await.unwrap();
    // Single use; the other device at either end is no party to it.
    assert_eq!(
        pipes.claim_sink(pipe.id(), "b").err(),
        Some(PipeRefusal::NotFound)
    );
    let second = pipes.mint(Some("a"), Some("b"));
    assert_eq!(
        pipes.claim_sink(second.id(), "a").err(),
        Some(PipeRefusal::NotFound)
    );
    assert_eq!(
        pipes.claim_source(second.id(), "b").err(),
        Some(PipeRefusal::NotFound)
    );
    let _sink = pipes.claim_sink(second.id(), "b").unwrap();
    assert_eq!(
        pipes.claim_sink(second.id(), "b").err(),
        Some(PipeRefusal::AlreadyConnected)
    );
    pipes.fail(second.id(), "test over");
    assert_eq!(
        second.done().await.unwrap_err().to_string(),
        "pipe failed: test over"
    );
}

#[tokio::test(flavor = "local")]
async fn this_process_reads_a_put_and_feeds_a_get_one_chunk_at_a_time() {
    let pipes = Pipes::new(Duration::from_secs(5));
    let inbound = pipes.from_device("a");
    let reader = inbound.reader().unwrap();
    let source = pipes.claim_source(inbound.id(), "a").unwrap();
    let put = tokio::task::spawn_local(source.pump(chunks(&["hello, ", "world"])));
    assert_eq!(collect(reader.into_stream()).await, b"hello, world");
    put.await.unwrap().unwrap();
    inbound.done().await.unwrap();

    let outbound = pipes.to_device("b");
    let mut writer = outbound.writer().unwrap();
    let wrote = Rc::new(RefCell::new(Vec::new()));
    let producer = {
        let wrote = wrote.clone();
        tokio::task::spawn_local(async move {
            for word in ["one ", "two ", "three"] {
                writer
                    .write(Bytes::from_static(word.as_bytes()))
                    .await
                    .unwrap();
                wrote.borrow_mut().push(word);
            }
            writer.end();
        })
    };
    for _ in 0..10 {
        tokio::task::yield_now().await;
    }
    // One chunk may be in flight; the next write waits for the sink.
    assert!(wrote.borrow().len() <= 1, "{:?}", wrote.borrow());
    let mut sink = pipes.claim_sink(outbound.id(), "b").unwrap();
    sink.source_arrived().await.unwrap();
    assert_eq!(collect(sink.into_stream()).await, b"one two three");
    producer.await.unwrap();
    outbound.done().await.unwrap();

    // A pipe with both ends in this process works the same way.
    let local = pipes.mint(None, None);
    let mut writer = local.writer().unwrap();
    let reader = local.reader().unwrap();
    tokio::task::spawn_local(async move {
        writer.write(Bytes::from_static(b"local")).await.unwrap();
        writer.end();
    });
    assert_eq!(collect(reader.into_stream()).await, b"local");
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn a_missing_end_times_out_a_lost_device_fails_its_pipes_an_early_reader_drains() {
    let pipes = Pipes::new(Duration::from_millis(200));
    let lonely = pipes.mint(Some("a"), Some("b"));
    assert!(
        lonely
            .done()
            .await
            .unwrap_err()
            .to_string()
            .contains("an end never arrived")
    );
    assert_eq!(
        pipes.claim_source(lonely.id(), "a").err(),
        Some(PipeRefusal::NotFound)
    );

    let dropped = pipes.mint(Some("a"), Some("b"));
    let mut waiting = pipes.claim_sink(dropped.id(), "b").unwrap();
    pipes.device_gone("a");
    let refusal = waiting.source_arrived().await.unwrap_err().to_string();
    assert!(refusal.contains("device a disconnected"), "{refusal}");

    // The reader takes one chunk and leaves: the PUT is answered as drained.
    let early = pipes.from_device("a");
    let mut reader = early.reader().unwrap();
    let source = pipes.claim_source(early.id(), "a").unwrap();
    let big: Vec<_> = (0..64)
        .map(|_| Ok::<_, Infallible>(Bytes::from(vec![0; 64 * 1024])))
        .collect();
    let put = tokio::task::spawn_local(source.pump(stream::iter(big)));
    assert!(!reader.next().await.unwrap().unwrap().is_empty());
    drop(reader);
    put.await.unwrap().unwrap();
    early.done().await.unwrap();
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn the_arrival_window_ends_once_both_ends_are_there_however_quiet() {
    let pipes = Pipes::new(Duration::from_millis(40));
    let pipe = pipes.mint(None, None);
    let mut writer = pipe.writer().unwrap();
    let mut reader = pipe.reader().unwrap();
    tokio::time::sleep(Duration::from_millis(80)).await;
    writer.write(Bytes::from_static(b"late")).await.unwrap();
    writer.end();
    assert_eq!(reader.next().await.unwrap().unwrap(), "late");
    assert!(reader.next().await.is_none());
    pipe.done().await.unwrap();

    // Device ends connect once and stay after the window.
    let pipe = pipes.mint(Some("a"), Some("b"));
    let (sender, body) = body();
    let source = pipes.claim_source(pipe.id(), "a").unwrap();
    let put = tokio::task::spawn_local(source.pump(body));
    let mut sink = pipes.claim_sink(pipe.id(), "b").unwrap();
    assert_eq!(
        pipes.claim_sink(pipe.id(), "b").err(),
        Some(PipeRefusal::AlreadyConnected)
    );
    assert_eq!(
        pipes.claim_source(pipe.id(), "a").err(),
        Some(PipeRefusal::AlreadyConnected)
    );
    tokio::time::sleep(Duration::from_millis(80)).await;
    sender.send(Ok(Bytes::from_static(b"late"))).await.unwrap();
    drop(sender);
    sink.source_arrived().await.unwrap();
    assert_eq!(collect(sink.into_stream()).await, b"late");
    put.await.unwrap().unwrap();
    pipe.done().await.unwrap();
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn handing_a_held_source_to_a_device_waits_for_that_device() {
    let pipes = Pipes::new(Duration::from_millis(40));
    let pipe = pipes.to_device("b");
    pipe.hold_source().unwrap();
    let mut sink = pipes.claim_sink(pipe.id(), "b").unwrap();
    pipe.source_from("a").unwrap();
    assert!(
        pipe.done()
            .await
            .unwrap_err()
            .to_string()
            .contains("an end never arrived")
    );
    assert!(sink.source_arrived().await.is_err());
    assert_eq!(pipe.writer().err(), Some(PipeError::Settled));
}

#[tokio::test(flavor = "local")]
async fn a_failure_interrupts_a_quiet_body_and_a_waiting_read() {
    let pipes = Pipes::new(Duration::from_secs(5));
    let pipe = pipes.mint(Some("a"), Some("b"));
    let (_sender, body) = body();
    let source = pipes.claim_source(pipe.id(), "a").unwrap();
    let put = tokio::task::spawn_local(source.pump(body));
    let mut sink = pipes.claim_sink(pipe.id(), "b").unwrap();
    sink.source_arrived().await.unwrap();
    let mut response = sink.into_stream();
    let read = tokio::task::spawn_local(async move { response.next().await });
    tokio::task::yield_now().await;
    pipes.fail(pipe.id(), "cancelled");
    let read = read.await.unwrap().unwrap().unwrap_err();
    assert_eq!(read.to_string(), "pipe failed: cancelled");
    assert_eq!(
        put.await.unwrap().unwrap_err().to_string(),
        "pipe failed: cancelled"
    );
    assert!(pipe.done().await.is_err());
}

#[tokio::test(flavor = "local")]
async fn a_device_report_counts_only_from_an_end_and_only_while_open() {
    let pipes = Pipes::new(Duration::from_secs(5));
    let pipe = pipes.mint(Some("a"), Some("b"));
    assert!(!pipes.fail_from_device(pipe.id(), "unrelated", "unauthorized failure"));
    let mut sink = pipes.claim_sink(pipe.id(), "b").unwrap();
    assert!(pipes.fail_from_device(pipe.id(), "a", "upload refused"));
    assert!(
        sink.source_arrived()
            .await
            .unwrap_err()
            .to_string()
            .contains("upload refused")
    );
    // Once over, the other end's failure is the consequence, not news.
    assert!(!pipes.fail_from_device(pipe.id(), "b", "later failure"));
    assert!(
        pipe.done()
            .await
            .unwrap_err()
            .to_string()
            .contains("upload refused")
    );
}

#[tokio::test(flavor = "local")]
async fn an_empty_upload_settles_both_ends() {
    let pipes = Pipes::new(Duration::from_secs(5));
    let pipe = pipes.mint(Some("a"), Some("b"));
    let source = pipes.claim_source(pipe.id(), "a").unwrap();
    let put = tokio::task::spawn_local(source.pump(chunks(&[])));
    let mut sink = pipes.claim_sink(pipe.id(), "b").unwrap();
    sink.source_arrived().await.unwrap();
    assert_eq!(collect(sink.into_stream()).await, b"");
    put.await.unwrap().unwrap();
    pipe.done().await.unwrap();
}

#[tokio::test(flavor = "local")]
async fn an_end_that_goes_away_before_the_end_fails_the_pipe() {
    let pipes = Pipes::new(Duration::from_secs(5));
    // A GET that leaves before the source arrived.
    let pipe = pipes.mint(Some("a"), Some("b"));
    drop(pipes.claim_sink(pipe.id(), "b").unwrap());
    assert_eq!(
        pipe.done().await.unwrap_err().to_string(),
        "pipe failed: sink HTTP request disconnected"
    );
    // A response cut short.
    let pipe = pipes.mint(None, Some("b"));
    let mut writer = pipe.writer().unwrap();
    let mut sink = pipes.claim_sink(pipe.id(), "b").unwrap();
    sink.source_arrived().await.unwrap();
    let mut response = sink.into_stream();
    tokio::task::spawn_local(async move {
        // The pipe fails under the writer when the response goes.
        let _ = writer.write(Bytes::from_static(b"first")).await;
        let _ = writer.write(Bytes::from_static(b"second")).await;
    });
    assert_eq!(response.next().await.unwrap().unwrap(), "first");
    drop(response);
    assert_eq!(
        pipe.done().await.unwrap_err().to_string(),
        "pipe failed: sink HTTP response disconnected before EOF"
    );
    // A PUT whose request went away mid-body.
    let pipe = pipes.mint(Some("a"), None);
    let _reader = pipe.reader().unwrap();
    let (_sender, body) = body();
    let source = pipes.claim_source(pipe.id(), "a").unwrap();
    let put = tokio::task::spawn_local(source.pump(body));
    tokio::task::yield_now().await;
    put.abort();
    assert_eq!(
        pipe.done().await.unwrap_err().to_string(),
        "pipe failed: source HTTP request disconnected"
    );
    // A writer dropped before its end: the sink never takes the cut stream
    // for a whole one.
    let pipe = pipes.mint(None, None);
    let writer = pipe.writer().unwrap();
    let mut reader = pipe.reader().unwrap();
    drop(writer);
    assert!(reader.next().await.unwrap().is_err());
}

#[tokio::test(flavor = "local")]
async fn closing_fails_every_pipe_and_every_later_one() {
    let pipes = Pipes::new(Duration::from_secs(5));
    let open = pipes.mint(Some("a"), None);
    pipes.close().await;
    assert_eq!(
        open.done().await.unwrap_err().to_string(),
        "pipe failed: backend shutting down"
    );
    let late = pipes.mint(None, Some("b"));
    assert_eq!(
        late.done().await.unwrap_err().to_string(),
        "pipe failed: backend shutting down"
    );
}
