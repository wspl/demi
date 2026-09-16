#![cfg(feature = "test-fixtures")]

use demi_command_service::protocol::{Invocation, PackageArtifact, PackageDescriptor, Record};
use demi_runner::commands::{
    cache::{ArtifactResolver, ArtifactSource, RuntimeError},
    native::{Services, target},
};
use futures_util::future::BoxFuture;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
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

#[tokio::test]
async fn conversation_state_retains_ownerless_services_and_release_joins_all_residents() {
    tokio::time::timeout(Duration::from_secs(30), async {
        let root = tempfile::tempdir().unwrap();
        let services = Services::new(root.path().join("cache"), target().into(), root.path().into(), BTreeMap::new()).await.unwrap();
        let bytes = tokio::fs::read(env!("CARGO_BIN_EXE_demi-native-fixture")).await.unwrap();
        let mut clients = Vec::new();
        // Distinct artifact digests make two actual resident processes.
        for index in 0..2 {
            let mut artifact_bytes = bytes.clone();
            artifact_bytes.extend(std::iter::repeat_n(0, index));
            let path = root.path().join(format!("fixture-{index}"));
            tokio::fs::write(&path, &artifact_bytes).await.unwrap();
            let descriptor = PackageDescriptor {
                id: format!("fixture-{index}"), version: "1.0.0".into(), protocol_version: 1,
                operations: ["where", "echo", "first", "spin", "result", "retain"].map(String::from).to_vec(),
                targets: BTreeMap::from([(target().into(), PackageArtifact {
                    sha256: format!("{:x}", Sha256::digest(&artifact_bytes)), size: artifact_bytes.len() as u64,
                })]),
            };
            let resolver = Arc::new(Local(path));
            let client = services.acquire(&descriptor, resolver.clone(), &CancellationToken::new()).await.unwrap();
            for conversation in ["one", "two"] {
                let (mut input, mut output) = client.invoke(&Invocation {
                    conversation: conversation.into(), caller: "node".into(),
                    operation: "retain".into(), invocation_id: format!("{index}-{conversation}"),
                    args: serde_json::json!({}), cwd: root.path().to_string_lossy().into_owned(),
                    env: BTreeMap::new(), edits: None, json: None,
                }).await.unwrap();
                input.end().unwrap();
                assert!(matches!(output.next().await.unwrap(), Some(Record::Completion(value)) if value.exit_code == 0));
                assert!(output.next().await.unwrap().is_none());
            }
            services.retain(&HashSet::new()).await;
            let retained = services.acquire(&descriptor, resolver, &CancellationToken::new()).await.unwrap();
            assert!(Arc::ptr_eq(&client, &retained));
            clients.push(client);
        }
        services.release_conversation("unknown").await.unwrap();
        services.release_conversation("one").await.unwrap();
        services.retain(&HashSet::new()).await;
        for client in &clients { assert!(client.info().await.is_ok()); }
        services.release_conversation("two").await.unwrap();
        services.retain(&HashSet::new()).await;
        for client in &clients { assert!(client.info().await.is_err()); }
        services.release_conversation("two").await.unwrap();
        services.close().await;
    }).await.unwrap();
}
