use std::{cell::RefCell, convert::Infallible, rc::Rc, time::Duration};

use bytes::Bytes;
use demi_backend_remote_host::{PipeError, PipeRefusal, Pipes};
use futures_util::{StreamExt, stream};
use demi_runner_protocol::wire::PipeFrame;
use tokio::sync::mpsc;

use crate::Framed;

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
        Result<Bytes, demi_backend_remote_host::PipeFailure>,
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
    let sink = pipes.claim_sink(pipe.id(), "b").unwrap();
    let source = pipes.claim_source(pipe.id(), "a").unwrap();
    let put = tokio::task::spawn_local(source.pump(stream::iter(parts)));
    assert_eq!(Framed::new(sink).read().await, (payload, Ok(())));
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
    let sink = pipes.claim_sink(outbound.id(), "b").unwrap();
    assert_eq!(Framed::new(sink).read().await, (b"one two three".to_vec(), Ok(())));
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
    let waiting = pipes.claim_sink(dropped.id(), "b").unwrap();
    pipes.device_gone("a");
    let refusal = Framed::new(waiting).read().await.1.unwrap_err();
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

/// A forwarded connection's pipes have its 10 s deadline, not the pipes'
/// window (`crates/backend-user-shard/src/shard/port_forwards.rs`).
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_pipe_minted_with_a_deadline_of_its_own_fails_when_its_ends_miss_it() {
    let pipes = Pipes::new(Duration::from_secs(120));
    let started = tokio::time::Instant::now();
    let lonely = pipes.mint_within(Some("a"), Some("b"), Duration::from_secs(10));
    let _source = pipes.claim_source(lonely.id(), "a").unwrap();
    let failed = lonely.done().await.unwrap_err().to_string();
    assert!(failed.contains("an end never arrived"), "{failed}");
    assert_eq!(started.elapsed(), Duration::from_secs(10));
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
    let sink = pipes.claim_sink(pipe.id(), "b").unwrap();
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
    assert_eq!(Framed::new(sink).read().await, (b"late".to_vec(), Ok(())));
    put.await.unwrap().unwrap();
    pipe.done().await.unwrap();
}

#[tokio::test(flavor = "local", start_paused = true)]
async fn handing_a_held_source_to_a_device_waits_for_that_device() {
    let pipes = Pipes::new(Duration::from_millis(40));
    let pipe = pipes.to_device("b");
    pipe.hold_source().unwrap();
    let sink = pipes.claim_sink(pipe.id(), "b").unwrap();
    pipe.source_from("a").unwrap();
    assert!(
        pipe.done()
            .await
            .unwrap_err()
            .to_string()
            .contains("an end never arrived")
    );
    assert!(Framed::new(sink).read().await.1.is_err());
    assert_eq!(pipe.writer().err(), Some(PipeError::Settled));
}

/// A sink's response goes out at once and keeps itself alive while the
/// pipe says nothing: an idle frame after each 30 s without a frame, so an
/// edge that ends a response silent for 100 s, as Cloudflare's does, keeps
/// it; its end frame carries the cause the device reports. Before, the
/// response sent nothing until the first byte, and Cloudflare ended a quiet
/// `demi shell status --wait 5m` with `pipe refused (524)`.
#[tokio::test(flavor = "local", start_paused = true)]
async fn a_quiet_sink_sends_an_idle_frame_every_30_seconds_and_ends_with_its_cause() {
    let pipes = Pipes::new(Duration::from_secs(600));
    let pipe = pipes.mint(Some("a"), Some("b"));
    let started = tokio::time::Instant::now();
    let mut response = Framed::new(pipes.claim_sink(pipe.id(), "b").unwrap());
    for idle in 1..=3 {
        assert_eq!(response.frame().await, Some(PipeFrame::Idle));
        assert_eq!(started.elapsed(), Duration::from_secs(30 * idle));
    }
    pipes.fail(pipe.id(), "the job's device went away");
    assert_eq!(
        response.frame().await,
        Some(PipeFrame::End(Err("the job's device went away".to_owned())))
    );
    assert_eq!(response.frame().await, None);
}

#[tokio::test(flavor = "local")]
async fn a_failure_interrupts_a_quiet_body_and_a_waiting_read() {
    let pipes = Pipes::new(Duration::from_secs(5));
    let pipe = pipes.mint(Some("a"), Some("b"));
    let (_sender, body) = body();
    let source = pipes.claim_source(pipe.id(), "a").unwrap();
    let put = tokio::task::spawn_local(source.pump(body));
    let sink = pipes.claim_sink(pipe.id(), "b").unwrap();
    let read = tokio::task::spawn_local(Framed::new(sink).read());
    tokio::task::yield_now().await;
    pipes.fail(pipe.id(), "cancelled");
    assert_eq!(read.await.unwrap(), (Vec::new(), Err("cancelled".to_owned())));
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
    let sink = pipes.claim_sink(pipe.id(), "b").unwrap();
    assert!(pipes.fail_from_device(pipe.id(), "a", "upload refused"));
    let (_, end) = Framed::new(sink).read().await;
    assert!(end.unwrap_err().contains("upload refused"));
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
    let sink = pipes.claim_sink(pipe.id(), "b").unwrap();
    assert_eq!(Framed::new(sink).read().await, (Vec::new(), Ok(())));
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
    let sink = pipes.claim_sink(pipe.id(), "b").unwrap();
    let mut response = Framed::new(sink);
    tokio::task::spawn_local(async move {
        // The pipe fails under the writer when the response goes.
        let _ = writer.write(Bytes::from_static(b"first")).await;
        let _ = writer.write(Bytes::from_static(b"second")).await;
    });
    assert_eq!(
        response.frame().await,
        Some(PipeFrame::Data(Bytes::from_static(b"first")))
    );
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
