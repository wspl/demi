#![cfg(unix)]

use std::{collections::BTreeMap, process::Stdio, time::Duration};

use demi_command_service::{
    Client,
    protocol::{Completion, Invocation, Record},
};
use tokio::process::Command;

async fn call(
    client: &Client,
    cwd: &str,
    operation: &str,
    args: serde_json::Value,
) -> (Completion, Vec<u8>, Vec<u8>) {
    let (mut input, mut output) = client
        .invoke(&Invocation {
            operation: operation.into(),
            invocation_id: operation.into(),
            args,
            cwd: cwd.into(),
            env: BTreeMap::new(),
        })
        .await
        .unwrap();
    input.end().unwrap();
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut completion = None;
    while let Some(record) = output.next().await.unwrap() {
        match record {
            Record::Stdout(bytes) => stdout.extend_from_slice(&bytes),
            Record::Stderr(bytes) => stderr.extend_from_slice(&bytes),
            Record::Completion(value) => completion = Some(value),
        }
    }
    (completion.unwrap(), stdout, stderr)
}

#[tokio::test]
async fn resident_executable_runs_all_builtin_file_operations() {
    tokio::time::timeout(Duration::from_secs(15), async {
        let root = tempfile::tempdir().unwrap();
        let cwd = root.path().to_str().unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_demi-commands"))
            .arg("--command-service")
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit())
            .kill_on_drop(true).spawn().unwrap();
        let pid = child.id().unwrap();
        let io = tokio::io::join(child.stdout.take().unwrap(), child.stdin.take().unwrap());
        let (client, connection) = Client::connect(io).await.unwrap();
        let driver = tokio::spawn(connection);
        assert_eq!(client.info().await.unwrap().operations.len(), 4);
        let (result, output, _) = call(&client, cwd, "file.create", serde_json::json!({"path":"nested/a.txt", "content":"alpha\nbeta\n"})).await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(output, b"Created nested/a.txt\n");
        let (result, _, _) = call(&client, cwd, "file.create", serde_json::json!({"path":"nested/a.txt", "content":"overwrite"})).await;
        assert_eq!(result.exit_code, 1);
        let (result, _, _) = call(&client, cwd, "file.edit", serde_json::json!({"path":"nested/a.txt", "old":"beta", "new":"gamma"})).await;
        assert_eq!(result.exit_code, 0);
        let patch = "--- a/nested/a.txt\n+++ b/nested/a.txt\n@@ -1,2 +1,2 @@\n alpha\n-gamma\n+delta\n--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+created\n";
        let (result, output, error) = call(&client, cwd, "file.patch", serde_json::json!({"patch":patch})).await;
        assert_eq!(result.exit_code, 0, "{}", String::from_utf8_lossy(&error));
        assert_eq!(output, b"Patched 2 file(s)\n");
        let (_, output, _) = call(&client, cwd, "file.read", serde_json::json!({"path":"nested/a.txt"})).await;
        assert_eq!(output, b"alpha\ndelta\n");
        let binary = [0, 255, 10, 13, 128];
        std::fs::write(root.path().join("image.bin"), binary).unwrap();
        let (result, output, _) = call(&client, cwd, "file.read", serde_json::json!({"path":"image.bin"})).await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(output, binary);
        assert_eq!(child.id(), Some(pid));
        client.shutdown().await.unwrap();
        let status = child.wait().await.unwrap();
        drop(client);
        driver.await.unwrap().unwrap();
        assert!(status.success());
    }).await.unwrap();
}

#[tokio::test]
async fn verified_artifact_launches_and_retires_through_runtime() {
    use demi_command_runtime::{
        ArtifactCache, ArtifactResolver, ArtifactSource, ResidentService, RuntimeError,
    };
    use demi_command_service::protocol::{PackageArtifact, PackageDescriptor, TARGETS};
    use futures_util::future::BoxFuture;
    use sha2::{Digest, Sha256};
    use std::sync::Arc;
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
    tokio::time::timeout(Duration::from_secs(15), async {
        let root = tempfile::tempdir().unwrap();
        let bytes = tokio::fs::read(env!("CARGO_BIN_EXE_demi-commands")).await.unwrap();
        let artifact = PackageArtifact { sha256: format!("{:x}", Sha256::digest(&bytes)), size: bytes.len() as u64 };
        // A catalog fixture; this test executes only the current host target.
        let package = PackageDescriptor::parse(serde_json::json!({
            "id":"demicodes.demi", "version":"test", "protocolVersion":1,
            "operations":["file.create", "file.read", "file.edit", "file.patch"],
            "targets": TARGETS.iter().map(|target| (target.to_string(), serde_json::to_value(&artifact).unwrap())).collect::<BTreeMap<_, _>>()
        })).unwrap();
        let cache = ArtifactCache::new(root.path().join("cache")).await.unwrap();
        let executable = cache.acquire(artifact, Arc::new(Local), &CancellationToken::new()).await.unwrap();
        let service = ResidentService::start(&executable, &package, root.path(), &BTreeMap::new()).await.unwrap();
        let pid = service.pid();
        let (result, _, _) = call(service.client(), root.path().to_str().unwrap(), "file.create", serde_json::json!({"path":"through-cache", "content":"hello"})).await;
        assert_eq!(result.exit_code, 0);
        assert_eq!(std::fs::read(root.path().join("through-cache")).unwrap(), b"hello");
        assert_eq!(service.pid(), pid);
        service.shutdown().await.unwrap();
        cache.shutdown().await;
    }).await.unwrap();
}
