//! `sse_data` over a body delivered in chunks exactly as split.

use std::convert::Infallible;

use bytes::Bytes;
use demi_provider::wire::{SseError, sse_data};
use futures_util::{StreamExt, TryStreamExt, stream};

async fn frames(chunks: &[&[u8]]) -> Vec<String> {
    let body = stream::iter(
        chunks
            .iter()
            .map(|chunk| Ok::<_, Infallible>(Bytes::copy_from_slice(chunk)))
            .collect::<Vec<_>>(),
    );
    sse_data(body).try_collect().await.unwrap()
}

#[tokio::test]
async fn frames_follow_the_specification() {
    // CRLF line ends; the event name is not carried.
    assert_eq!(frames(&[b"event: delta\r\ndata: {\"a\":1}\r\n\r\n"]).await, ["{\"a\":1}"]);
    // Multi-line data joins with a newline.
    assert_eq!(frames(&[b"data: line one\ndata: line two\n\n"]).await, ["line one\nline two"]);
    // The [DONE] sentinel is a payload like any other.
    assert_eq!(frames(&[b"data: {\"a\":1}\n\n", b"data: [DONE]\n\n"]).await, ["{\"a\":1}", "[DONE]"]);
    // An empty body has no frames.
    assert!(frames(&[]).await.is_empty());
}

#[tokio::test]
async fn a_final_frame_without_its_blank_line_is_kept() {
    assert_eq!(frames(&[b"data: {\"a\":1}"]).await, ["{\"a\":1}"]);
    assert_eq!(frames(&[b"data: {\"a\":1}\n"]).await, ["{\"a\":1}"]);
}

#[tokio::test]
async fn frames_split_across_chunks_reassemble_including_multibyte_text() {
    let text = "data: héllo\n\n".as_bytes();
    // Split inside the two bytes of `é`.
    assert_eq!(frames(&[&text[..8], &text[8..]]).await, ["héllo"]);
    assert_eq!(frames(&[b"da", b"ta: x", b"\n", b"\n"]).await, ["x"]);
}

#[tokio::test]
async fn frames_without_data_are_not_events() {
    let body: &[&[u8]] = &[b": keep-alive\n\nevent: ping\n\ndata:\n\ndata: x\n\n"];
    assert_eq!(frames(body).await, ["x"]);
}

#[tokio::test]
async fn a_body_that_breaks_off_reports_its_error_after_the_frames_before_it() {
    let body = stream::iter([
        Ok(Bytes::from_static(b"data: first\n\n")),
        Err("connection reset"),
    ]);
    let items: Vec<_> = sse_data(body).collect().await;
    assert_eq!(items[0].as_ref().unwrap(), "first");
    assert!(matches!(items[1], Err(SseError::Transport("connection reset"))));
}
