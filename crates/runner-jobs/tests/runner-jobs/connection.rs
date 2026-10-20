//! What work under the runner asks the backend (`crates/backend-runners/src/publication.rs`): where an artifact
//! downloads from is answered once the backend has the artifact in its
//! store, which on a small server can take long.

use std::time::Duration;

use demi_command_protocol::{ArtifactLocation, ArtifactUrl};
use demi_runner_jobs::connection::{ConnectionHandle, Live, Question, Reach, Request};
use demi_runner_protocol::wire;
use tokio::sync::{mpsc, watch};
use tokio_util::sync::CancellationToken;

/// The first browser command after an upgrade on a small server: the backend
/// copies the executable into its store before it answers, which took 11
/// seconds and missed the runner's old 15-second deadline once the disk was
/// slower. The runner waits for the answer as long as its connection lives.
/// The clock is paused, so the backend's minute passes at once.
#[tokio::test(start_paused = true)]
async fn an_artifact_location_the_backend_answers_after_a_minute_still_arrives() {
    let (control, _frames) = mpsc::channel(1);
    let reach = watch::Sender::new(Reach::Connected(Live {
        control,
        closed: CancellationToken::new(),
    }));
    let (handle, mut requests) = ConnectionHandle::new(reach.subscribe());
    let owner = wire::ArtifactOwner::Stream(wire::StreamArtifactOwner {
        stream_id: "stream-1".into(),
    });
    let asked = tokio::spawn(async move { handle.locate(owner, "a".repeat(64)).await });
    let Some(Request::Ask {
        question: Question::Locate { reply, .. },
        ..
    }) = requests.recv().await
    else {
        panic!("the runner asks where the artifact is");
    };
    tokio::time::sleep(Duration::from_secs(60)).await;
    let location = ArtifactLocation::Url(ArtifactUrl {
        url: format!("http://127.0.0.1:3271/native-artifacts/{}", "a".repeat(64)),
        expires_at: None,
    });
    // An asker that gave up dropped its end; its result says why.
    let _ = reply.send(Ok(location.clone()));
    assert_eq!(asked.await.unwrap().unwrap(), location);
}
