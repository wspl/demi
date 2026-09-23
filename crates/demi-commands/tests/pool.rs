use demi_command_service::protocol::{PackageArtifact, PackageDescriptor, TARGETS};
use demi_runner::services::{
    ArtifactResolver, ArtifactSource, RuntimeError, ServiceRegistry, target,
};
use futures_util::future::BoxFuture;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio_util::sync::CancellationToken;

struct Local;
impl ArtifactResolver for Local {
    fn resolve<'a>(
        &'a self,
        _: &'a PackageArtifact,
        _: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(async {
            Ok(ArtifactSource::Local(
                env!("CARGO_BIN_EXE_demi-commands").into(),
            ))
        })
    }
}

#[tokio::test]
async fn concurrent_acquisition_shares_service_and_checks_every_descriptor() {
    // Two acquisitions verify the full debug executable, including symbol data.
    tokio::time::timeout(Duration::from_secs(60), async {
        let directory = tempfile::tempdir().unwrap();
        let bytes = tokio::fs::read(env!("CARGO_BIN_EXE_demi-commands")).await.unwrap();
        let artifact = PackageArtifact { sha256: format!("{:x}", Sha256::digest(&bytes)), size: bytes.len() as u64 };
        // Test-only descriptor: only this host's executable is launched.
        let package = PackageDescriptor::parse(serde_json::json!({
            "id":"demicodes.fixture", "version":"test", "protocolVersion":1,
            "operations":demi_command_service::Handler::operations(&demi_commands::DemiCommands::default()),
            "targets":TARGETS.iter().map(|target| (target.to_string(), serde_json::to_value(&artifact).unwrap())).collect::<BTreeMap<_, _>>()
        })).unwrap();
        let registry = ServiceRegistry::new(directory.path().join("cache"), directory.path().into(), BTreeMap::new()).await.unwrap();
        let services = registry.handle();
        let _lease = services.lease(package.targets[target()].sha256.clone());
        let resolver: Arc<dyn ArtifactResolver> = Arc::new(Local);
        let stop = CancellationToken::new();
        let first = services.acquire(&package, resolver.clone(), &stop);
        let second = services.acquire(&package, resolver.clone(), &stop);
        let (first, second) = tokio::join!(first, second);
        let (first, second) = (first.unwrap(), second.unwrap());
        assert!(first.client().info().await.is_ok());
        assert!(second.client().info().await.is_ok());
        let mut conflicting = package.clone();
        conflicting.operations.pop();
        assert!(matches!(&*services.acquire(&conflicting, resolver, &stop).await.err().unwrap(), RuntimeError::CatalogMismatch));
        registry.close().await;
        assert!(first.client().info().await.is_err());
    }).await.unwrap();
}
