//! The manifests a connection installs (`crates/runner-jobs/src/commands/mod.rs`; `crates/runner-command-packages/src/registry.rs`): a job waits only for the manifest its start names, and each
//! package keeps resident only the release the latest manifest naming it
//! names.

use std::{collections::BTreeMap, collections::BTreeSet, path::Path, path::PathBuf, sync::Arc};

use demi_command_protocol::{
    PackageArtifact, PackageDescriptor, host_target,
    testing::{FIXTURE_OPERATIONS, built_program},
};
use demi_runner_command_packages::{
    ArtifactResolver, ArtifactSource, Decision, RuntimeError, ServiceRegistry, testing::NoBackend,
};
use demi_runner_jobs::commands::contexts::{ContextPaths, Installed, Manifests};
use demi_runner_protocol::manifest::Manifest;
use futures_util::future::BoxFuture;
use sha2::{Digest, Sha256};
use tokio::{sync::broadcast, time::Duration};
use tokio_util::sync::CancellationToken;

/// Hands out a local copy of an artifact.
struct Local(PathBuf);

impl ArtifactResolver for Local {
    fn resolve<'a>(
        &'a self,
        _: &'a PackageArtifact,
        _: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(async move { Ok(ArtifactSource::Local(self.0.clone())) })
    }
}

/// A manifest whose one command is `name`, with `packages`.
fn manifest(name: &str, packages: impl IntoIterator<Item = PackageDescriptor>) -> Manifest {
    let tree = serde_json::json!({
        "name": name, "summary": "Fixture command", "kind": "rpc",
        "input": {"type": "object", "properties": {}}
    });
    Manifest::build([serde_json::from_value(tree).unwrap()], packages).unwrap()
}

/// Release `version` of the package `fixture`: the fixture service with
/// `variant` extra bytes, a distinct artifact.
async fn release(root: &Path, version: &str, variant: usize) -> (PackageDescriptor, PathBuf) {
    let mut bytes = tokio::fs::read(built_program("demi-native-fixture"))
        .await
        .unwrap();
    bytes.extend(std::iter::repeat_n(0, variant));
    let path = root.join(format!("fixture-{version}"));
    tokio::fs::write(&path, &bytes).await.unwrap();
    let descriptor = PackageDescriptor {
        id: "demi.fixture".into(),
        version: version.into(),
        protocol_version: 1,
        operations: FIXTURE_OPERATIONS.map(String::from).to_vec(),
        targets: BTreeMap::from([(
            host_target().into(),
            PackageArtifact {
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                size: bytes.len() as u64,
            },
        )]),
    };
    (descriptor, path)
}

/// Starts installing `manifest` on the connection `manifests` stands for.
fn install(
    manifests: &mut Manifests,
    paths: &ContextPaths,
    registry: &ServiceRegistry,
    manifest: &Manifest,
) -> impl Future<Output = (u64, std::io::Result<Installed>)> + use<> {
    manifests.install(
        serde_json::to_value(manifest).unwrap(),
        paths.clone(),
        registry.handle(),
        BTreeSet::new(),
    )
}

async fn registry(root: &Path) -> ServiceRegistry {
    ServiceRegistry::new(root.join("cache"), None, None, root.into(), BTreeMap::new())
        .await
        .unwrap()
}

/// The registry's next decision about the service of `digest`.
async fn decided(decisions: &mut broadcast::Receiver<(String, Decision)>, digest: &str) -> Decision {
    loop {
        let (about, decision) = decisions.recv().await.expect("the registry decides");
        if about == digest {
            return decision;
        }
    }
}

/// A root's and a subagent's manifests arrive one after the other and the
/// first one's install stalls: the job pinned to the second runs once the
/// second is installed. Cost: in process, a few milliseconds.
#[tokio::test]
async fn a_job_waits_only_for_the_manifest_it_names_while_another_install_stalls() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let registry = registry(root.path()).await;
        let paths = ContextPaths::new(root.path().join("manifests"), root.path().join("runner"))
            .await
            .unwrap();
        let mut manifests = Manifests::default();
        let stalled = install(&mut manifests, &paths, &registry, &manifest("root-only", []));
        let child = manifest("child-only", []);
        let installing = install(&mut manifests, &paths, &registry, &child);
        let pinned = manifests.pin(child.hash.clone());
        // The first install is never driven; the second completes.
        let (number, installed) = installing.await;
        manifests.installed(number, installed).unwrap();
        let pinned = tokio::time::timeout(Duration::from_secs(5), pinned.manifest())
            .await
            .expect("the job waits only for its own manifest")
            .unwrap();
        assert_eq!(pinned.hash, child.hash);
        drop(stalled);
        registry.close().await;
    })
    .await
    .unwrap();
}

/// After the catalog moves a package to a new release, the manifest that
/// names the new one takes the package's lease: the old release's service,
/// which no job holds, stops, though its manifest stays installed.
/// Cost: one fixture service process, about a second.
#[tokio::test]
async fn a_newer_release_of_a_package_takes_its_lease_and_the_older_service_stops() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let registry = registry(root.path()).await;
        let services = registry.handle();
        let mut decisions = registry.decisions();
        let paths = ContextPaths::new(root.path().join("manifests"), root.path().join("runner"))
            .await
            .unwrap();
        let (old, old_path) = release(root.path(), "1.0.0", 0).await;
        let (new, _) = release(root.path(), "1.1.0", 1).await;
        let old_digest = old.targets[host_target()].sha256.clone();
        let mut manifests = Manifests::default();
        for descriptor in [old.clone(), new] {
            let named = manifest("fixture", [descriptor.clone()]);
            let installing = install(&mut manifests, &paths, &registry, &named);
            let (number, installed) = installing.await;
            manifests.installed(number, installed).unwrap();
            if descriptor == old {
                // The old release's service runs; the installed manifest's
                // lease keeps it once the start's own ends.
                let resident = services
                    .acquire(
                        &old,
                        Arc::new(Local(old_path.clone())),
                        Arc::new(NoBackend),
                        &CancellationToken::new(),
                    )
                    .await
                    .unwrap();
                assert_eq!(decided(&mut decisions, &old_digest).await, Decision::Leased);
                drop(resident);
            }
        }
        // The fixture holds no conversation, so asked, it stops.
        assert_eq!(decided(&mut decisions, &old_digest).await, Decision::Asks);
        assert_eq!(decided(&mut decisions, &old_digest).await, Decision::Stops);
        drop(manifests);
        registry.close().await;
    })
    .await
    .unwrap();
}
