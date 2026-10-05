//! A local store end to end (`native-runtime.md` § Backend deployment
//! configuration). The harness writes the command packages the workspace
//! built as development releases of a server release, which the backend's
//! local store takes from the release's files when a runner first needs
//! them and serves itself (`support::Harness::with_file_package`), so every
//! scenario whose runner runs a native command, on a paired device or on the
//! Cloud, installs it from the backend. This file shows the store's own
//! promises: a runner installs a package from the backend's route and runs
//! it, and the route serves only what the catalog names; the device then
//! lists what its runner's cache holds, even while it is offline
//! (§ Installed artifacts).

use demi_command_protocol::host_target;
use demi_runner_protocol::wire::HostArtifact;
use demi_web_api_protocol::error::ErrorCode;
use reqwest::StatusCode;
use reqwest::header::{CACHE_CONTROL, CONTENT_ENCODING};

use crate::streams::{self, CONVERSATION};
use crate::support::{FIXTURE, Harness, eventually};

#[tokio::test]
async fn a_runner_installs_a_development_release_from_the_backend_which_serves_nothing_else() {
    let harness = Harness::new().with_native_fixture();
    let (backend, master, laptop) = streams::conversation(&harness).await;
    // The stream runs the fixture package's `echo`, whose program the
    // laptop's runner downloaded from the backend, over plain HTTP.
    let mut echo = streams::socket(&backend, &master, CONVERSATION, "echo").await;
    streams::answered(&mut echo).await;
    echo.close(None).await.unwrap();

    // The laptop's runner reports the program its cache now holds, which
    // the device keeps while the runner is away.
    let program = HostArtifact {
        package: FIXTURE.descriptor.id.clone(),
        name: "program".into(),
        version: FIXTURE.descriptor.version.clone(),
    };
    let device = laptop.id().to_owned();
    let lists = |online: bool| {
        let (backend, master, device, program) = (&backend, &master, &device, &program);
        move || async move {
            backend.devices(master).await.iter().any(|listed| {
                listed.id.as_str() == device
                    && listed.online == online
                    && listed.installed.contains(program)
            })
        }
    };
    eventually("the laptop lists the program it installed", lists(true)).await;
    drop(laptop);
    eventually("the offline laptop still lists it", lists(false)).await;

    // The route serves the loaded executable whole, as an immutable file,
    // in the content coding a runner decodes.
    let artifact = &FIXTURE.descriptor.targets[host_target()];
    let path = format!("/native-artifacts/{}", artifact.sha256);
    let served = backend.get(&path, None).await;
    assert_eq!(served.status, StatusCode::OK);
    assert!(
        served.body == std::fs::read(&FIXTURE.program).unwrap(),
        "the served bytes differ from the program"
    );
    assert_eq!(
        served.headers[CACHE_CONTROL],
        "public, max-age=31536000, immutable"
    );
    let undecoded = reqwest::Client::builder().no_zstd().build().unwrap();
    let encoded = undecoded
        .get(format!("{}{path}", backend.url))
        .send()
        .await
        .unwrap();
    assert_eq!(encoded.headers()[CONTENT_ENCODING], "zstd");
    assert!(encoded.bytes().await.unwrap().len() < artifact.size as usize);
    // A digest no loaded release carries, or no digest at all, is not there.
    for unknown in ["0".repeat(64), "demi-native-fixture".to_owned()] {
        let refused = backend
            .get(&format!("/native-artifacts/{unknown}"), None)
            .await;
        assert_eq!(
            refused.refusal(),
            (StatusCode::NOT_FOUND, ErrorCode::NotFound),
            "{unknown}"
        );
    }
    backend.close().await;
}
