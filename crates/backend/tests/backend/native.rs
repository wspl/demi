//! The development store end to end (`native-runtime.md` § Backend
//! deployment configuration). The harness writes the native packages the
//! workspace built as development releases, which the backend's local store
//! serves itself (`support::Harness::with_builtin_package`), so every
//! scenario whose runner runs a native command, on a paired device or on the
//! Cloud, installs it from the backend. This file shows the store's own
//! promises: a runner installs a package from the backend's route and runs
//! it, and the route serves only what the backend loaded.

use demi_command_service::protocol::host_target;
use demi_web_api::error::ErrorCode;
use reqwest::StatusCode;
use reqwest::header::{CACHE_CONTROL, CONTENT_LENGTH};

use crate::streams::{self, CONVERSATION};
use crate::support::{FIXTURE, Harness};

#[tokio::test]
async fn a_runner_installs_a_development_release_from_the_backend_which_serves_nothing_else() {
    let harness = Harness::new().with_native_fixture();
    let (backend, master, _laptop) = streams::conversation(&harness).await;
    // The stream runs the fixture package's `echo`, whose program the
    // laptop's runner downloaded from the backend, over plain HTTP.
    let mut echo = streams::socket(&backend, &master, CONVERSATION, "echo").await;
    streams::answered(&mut echo).await;
    echo.close(None).await.unwrap();

    // The route serves the loaded executable whole, as an immutable file.
    let artifact = &FIXTURE.descriptor.targets[host_target()];
    let served = backend.get(&format!("/native-artifacts/{}", artifact.sha256), None).await;
    assert_eq!(served.status, StatusCode::OK);
    assert!(served.body == std::fs::read(&FIXTURE.program).unwrap(), "the served bytes differ from the program");
    assert_eq!(served.headers[CONTENT_LENGTH], artifact.size.to_string().as_str());
    assert_eq!(served.headers[CACHE_CONTROL], "public, max-age=31536000, immutable");
    // A digest no loaded release carries, or no digest at all, is not there.
    for unknown in ["0".repeat(64), "demi-native-fixture".to_owned()] {
        let refused = backend.get(&format!("/native-artifacts/{unknown}"), None).await;
        assert_eq!(refused.refusal(), (StatusCode::NOT_FOUND, ErrorCode::NotFound), "{unknown}");
    }
    backend.close().await;
}
