use demi_command_service::protocol::{PackageArtifact, PackageDescriptor, TARGETS};
use demi_runner::commands::cache::{ArtifactResolver, ArtifactSource, RuntimeError};
use demi_runner::commands::native::{self, Services};
use futures_util::future::BoxFuture;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
    time::Duration,
};
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
        let pool = Services::new(directory.path().join("cache"), native::target().into(), directory.path().into(), BTreeMap::new()).await.unwrap();
        let resolver: Arc<dyn ArtifactResolver> = Arc::new(Local);
        let stop = CancellationToken::new();
        let first = pool.acquire(&package, resolver.clone(), &stop);
        let second = pool.acquire(&package, resolver.clone(), &stop);
        let (first, second) = tokio::join!(first, second);
        let first = first.unwrap();
        assert!(Arc::ptr_eq(&first, &second.unwrap()));
        let mut conflicting = package.clone();
        conflicting.operations.pop();
        assert!(matches!(&*pool.acquire(&conflicting, resolver.clone(), &stop).await.err().unwrap(), RuntimeError::CatalogMismatch));
        pool.retain(&HashSet::new()).await;
        let replacement = pool.acquire(&package, resolver, &stop).await.unwrap();
        assert!(!Arc::ptr_eq(&first, &replacement));
        pool.close().await;
        assert!(replacement.info().await.is_err());
    }).await.unwrap();
}
