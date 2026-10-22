//! `sse_data` over a body delivered in chunks exactly as split.

use std::convert::Infallible;

use bytes::Bytes;
use demi_provider_common::wire::sse_data;
use futures_util::{TryStreamExt, stream};

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
    assert_eq!(
        frames(&[b"event: delta\r\ndata: {\"a\":1}\r\n\r\n"]).await,
        ["{\"a\":1}"]
    );
    // Multi-line data joins with a newline.
    assert_eq!(
        frames(&[b"data: line one\ndata: line two\n\n"]).await,
        ["line one\nline two"]
    );
    // The [DONE] sentinel is a payload like any other.
    assert_eq!(
        frames(&[b"data: {\"a\":1}\n\n", b"data: [DONE]\n\n"]).await,
        ["{\"a\":1}", "[DONE]"]
    );
    // An empty body has no frames.
    assert!(frames(&[]).await.is_empty());
}

#[tokio::test]
async fn frames_split_across_chunks_reassemble_including_multibyte_text() {
    let text = "data: héllo\n\n".as_bytes();
    // Split inside the two bytes of `é`.
    assert_eq!(frames(&[&text[..8], &text[8..]]).await, ["héllo"]);
    assert_eq!(frames(&[b"da", b"ta: x", b"\n", b"\n"]).await, ["x"]);
}
