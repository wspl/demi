//! No native call is refused for how many others are in flight
//! (`runner.md` § Load).

use demi_command_protocol::{
    CommandCaller, CommandContext, CommandLocale, Invocation, PackageArtifact, PackageDescriptor,
    Record, host_target,
};
use demi_runner_command_packages::{
    ArtifactResolver, ArtifactSource, Resident, RuntimeError, ServiceLease, ServiceRegistry,
    testing::NoNumbers,
};
use futures_util::future::BoxFuture;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio_util::sync::CancellationToken;

struct Local(PathBuf);
impl ArtifactResolver for Local {
    fn resolve<'a>(
        &'a self,
        _: &'a PackageArtifact,
        _: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(async { Ok(ArtifactSource::Local(self.0.clone())) })
    }
}

fn command_context(conversation: &str) -> CommandContext {
    CommandContext {
        conversation: conversation.into(),
        caller: CommandCaller::agent(1),
        locale: CommandLocale {
            time_zone: "UTC".into(),
            languages: vec!["en-US".into()],
        },
    }
}

fn invocation(operation: &str, conversation: &str) -> Invocation {
    Invocation {
        context: command_context(conversation),
        operation: operation.into(),
        invocation_id: operation.into(),
        args: json!({}),
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        env: BTreeMap::new(),
        edits: None,
        json: None,
    }
}

/// A running fixture service and the lease that keeps it, as a job's context
/// would.
async fn resident(root: &Path) -> (ServiceRegistry, ServiceLease, Resident) {
    let services = ServiceRegistry::new(root.join("cache"), None, root.into(), BTreeMap::new())
        .await
        .unwrap();
    let bytes = tokio::fs::read(env!("CARGO_BIN_EXE_demi-native-fixture"))
        .await
        .unwrap();
    let path = root.join("fixture");
    tokio::fs::write(&path, &bytes).await.unwrap();
    let descriptor = PackageDescriptor {
        id: "fixture".into(),
        version: "1.0.0".into(),
        protocol_version: 1,
        operations: demi_command_protocol::testing::FIXTURE_OPERATIONS
            .map(String::from)
            .to_vec(),
        targets: BTreeMap::from([(
            host_target().into(),
            PackageArtifact {
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                size: bytes.len() as u64,
            },
        )]),
    };
    let lease = services
        .handle()
        .lease(descriptor.targets[host_target()].sha256.clone())
        .await;
    let resident = services
        .handle()
        .acquire(
            &descriptor,
            Arc::new(Local(path)),
            Arc::new(NoNumbers),
            &CancellationToken::new(),
        )
        .await
        .unwrap();
    (services, lease, resident)
}

/// Native calls held open, then a cancel and a conversation release at once:
/// the release is admitted, and every other call keeps running.
#[tokio::test(flavor = "multi_thread")]
async fn native_calls_are_never_turned_away() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let (services, _lease, resident) = resident(root.path()).await;
        let client = resident.client();
        let mut held = Vec::new();
        for _ in 0..128 {
            held.push(client.invoke(&invocation("echo", "running")).await.unwrap());
        }
        for attempt in 0..40 {
            let (mut input, _output) = held.remove(0);
            input.cancel();
            services
                .handle()
                .release_conversation(&format!("archived-{attempt}"))
                .await
                .unwrap();
            held.push(client.invoke(&invocation("echo", "running")).await.unwrap());
        }
        for (mut input, mut output) in held {
            input.end().unwrap();
            let mut completed = false;
            while let Some(record) = output.next().await.unwrap() {
                completed |= matches!(record, Record::Completion(value) if value.exit_code == 0);
            }
            assert!(completed);
        }
        assert!(client.info().await.is_ok());
        services.close().await;
    })
    .await
    .unwrap();
}
