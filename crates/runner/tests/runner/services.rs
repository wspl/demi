#![cfg(feature = "test-fixtures")]
//! The service registry (`native-runtime.md` § Keep a service resident): a
//! service stays while a lease or a conversation holds it, or while it cannot
//! say what it holds; a failed release retires it; a service that fails
//! reports its exit status and the end of its standard error. A service
//! starts from the Host image's copy of its executable when that copy
//! matches (§ Preinstalled executables).

use demi_command_service::protocol::{
    CommandCaller, CommandContext, CommandLocale, Invocation, PackageArtifact, PackageDescriptor,
    Record,
};
use demi_runner::host_log::{self, Query};
use demi_runner::services::{
    ArtifactResolver, ArtifactSource, Resident, RuntimeError, ServiceHandle, ServiceRegistry,
    target,
};
use futures_util::future::BoxFuture;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;
use tracing_subscriber::{Layer as _, filter::LevelFilter, layer::SubscriberExt as _};

/// Hands out a local copy of the fixture, counting the requests; with `gate`
/// set, each request waits for it.
struct Local {
    path: PathBuf,
    calls: AtomicUsize,
    gate: Option<(Notify, Notify)>,
}

impl ArtifactResolver for Local {
    fn resolve<'a>(
        &'a self,
        _: &'a PackageArtifact,
        _: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<ArtifactSource, RuntimeError>> {
        Box::pin(async move {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if let Some((started, proceed)) = &self.gate {
                started.notify_one();
                proceed.notified().await;
            }
            Ok(ArtifactSource::Local(self.path.clone()))
        })
    }
}

/// The fixture as its own artifact: `variant` extra bytes make a distinct
/// digest, and so a distinct service.
async fn fixture(root: &Path, variant: usize) -> (PackageDescriptor, PathBuf) {
    let mut bytes = tokio::fs::read(env!("CARGO_BIN_EXE_demi-native-fixture"))
        .await
        .unwrap();
    bytes.extend(std::iter::repeat_n(0, variant));
    let path = root.join(format!("fixture-{variant}"));
    tokio::fs::write(&path, &bytes).await.unwrap();
    let descriptor = PackageDescriptor {
        id: format!("fixture-{variant}"),
        version: "1.0.0".into(),
        protocol_version: 1,
        operations: ["where", "echo", "first", "spin", "result", "retain", "crash"]
            .map(String::from)
            .to_vec(),
        targets: BTreeMap::from([(
            target().into(),
            PackageArtifact {
                sha256: format!("{:x}", Sha256::digest(&bytes)),
                size: bytes.len() as u64,
            },
        )]),
    };
    (descriptor, path)
}

fn local(path: PathBuf) -> Arc<Local> {
    Arc::new(Local {
        path,
        calls: AtomicUsize::new(0),
        gate: None,
    })
}

fn digest(descriptor: &PackageDescriptor) -> String {
    descriptor.targets[target()].sha256.clone()
}

async fn registry(root: &Path) -> ServiceRegistry {
    ServiceRegistry::new(root.join("cache"), None, root.into(), BTreeMap::new())
        .await
        .unwrap()
}

/// Runs `operation` for `conversation` and returns its completion's exit code.
async fn call(resident: &Resident, operation: &str, conversation: &str) -> u8 {
    let (mut input, mut output) = resident
        .client()
        .invoke(&Invocation {
            context: CommandContext {
                conversation: conversation.into(),
                caller: CommandCaller::agent("node"),
                locale: CommandLocale {
                    time_zone: "UTC".into(),
                    languages: vec!["en-US".into()],
                },
            },
            operation: operation.into(),
            invocation_id: format!("{operation}-{conversation}"),
            args: serde_json::json!({}),
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            env: BTreeMap::new(),
            edits: None,
            json: None,
        })
        .await
        .unwrap();
    input.end().unwrap();
    let mut exit = None;
    while let Some(record) = output.next().await.unwrap() {
        if let Record::Completion(completion) = record {
            exit = Some(completion.exit_code);
        }
    }
    exit.expect("a completion")
}

/// Waits until the service behind `resident` has stopped.
async fn stopped(resident: &Resident) {
    tokio::time::timeout(Duration::from_secs(60), async {
        while resident.client().info().await.is_ok() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the service stops");
}

/// Gives the registry time to decide, then checks the service still answers.
async fn stays(resident: &Resident) {
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(resident.client().info().await.is_ok(), "the service stays");
}

async fn acquire(services: &ServiceHandle, descriptor: &PackageDescriptor, resolver: Arc<Local>) -> Resident {
    services
        .acquire(descriptor, resolver, &CancellationToken::new())
        .await
        .unwrap()
}

#[tokio::test]
async fn a_service_without_leases_stays_while_it_holds_a_conversation() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let registry = registry(root.path()).await;
        let services = registry.handle();
        let (descriptor, path) = fixture(root.path(), 0).await;
        let lease = services.lease(digest(&descriptor)).await;
        let resident = acquire(&services, &descriptor, local(path)).await;
        for conversation in ["one", "two"] {
            assert_eq!(call(&resident, "retain", conversation).await, 0);
        }
        // Its last lease ends; it still holds two conversations.
        drop(lease);
        stays(&resident).await;
        services.release_conversation("unknown").await.unwrap();
        services.release_conversation("one").await.unwrap();
        stays(&resident).await;
        // The last conversation goes, and nothing holds the service.
        services.release_conversation("two").await.unwrap();
        stopped(&resident).await;
        // Releasing again starts nothing and is harmless.
        services.release_conversation("two").await.unwrap();
        registry.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_lease_keeps_a_service_that_holds_nothing() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let registry = registry(root.path()).await;
        let services = registry.handle();
        let (descriptor, path) = fixture(root.path(), 0).await;
        let resolver = local(path);
        let lease = services.lease(digest(&descriptor)).await;
        let resident = acquire(&services, &descriptor, resolver.clone()).await;
        stays(&resident).await;
        // A second caller reaches the same process.
        let again = acquire(&services, &descriptor, resolver.clone()).await;
        assert_eq!(call(&again, "retain", "shared").await, 0);
        services.release_conversation("shared").await.unwrap();
        assert!(resident.client().info().await.is_ok());
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
        drop(lease);
        stopped(&resident).await;
        registry.close().await;
    })
    .await
    .unwrap();
}

/// A status check that fails keeps the service (`native-runtime.md` § Keep a
/// service resident): a service that cannot say what it holds is not one
/// that holds nothing.
#[tokio::test]
async fn a_service_that_cannot_say_what_it_holds_stays() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let registry = registry(root.path()).await;
        let services = registry.handle();
        let (descriptor, path) = fixture(root.path(), 0).await;
        let lease = services.lease(digest(&descriptor)).await;
        let resident = acquire(&services, &descriptor, local(path)).await;
        assert_eq!(call(&resident, "retain", "unanswerable").await, 0);
        drop(lease);
        stays(&resident).await;
        services.release_conversation("unanswerable").await.unwrap();
        stopped(&resident).await;
        registry.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn a_failed_release_retires_the_service_and_the_next_caller_gets_a_new_one() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let registry = registry(root.path()).await;
        let services = registry.handle();
        let (descriptor, path) = fixture(root.path(), 0).await;
        let resolver = local(path);
        let _lease = services.lease(digest(&descriptor)).await;
        let resident = acquire(&services, &descriptor, resolver.clone()).await;
        // The service reports its failure, or dies before its answer
        // arrives; either way the release fails and the service retires.
        assert!(services.release_conversation("fail").await.is_err());
        // The release is answered once the service has gone.
        assert!(resident.client().info().await.is_err());
        let next = acquire(&services, &descriptor, resolver).await;
        assert!(next.client().info().await.is_ok());
        registry.close().await;
    })
    .await
    .unwrap();
}

/// A call that fails because its service died reports the exit status and
/// the end of the service's standard error (`native-runtime.md` § Invocation
/// protocol); the next caller starts a new service.
#[tokio::test]
async fn a_service_that_dies_reports_its_status_and_standard_error() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let registry = registry(root.path()).await;
        let services = registry.handle();
        let (descriptor, path) = fixture(root.path(), 0).await;
        let resolver = local(path);
        let _lease = services.lease(digest(&descriptor)).await;
        let mut resident = acquire(&services, &descriptor, resolver.clone()).await;
        let invocation = Invocation {
            context: CommandContext {
                conversation: "c".into(),
                caller: CommandCaller::agent("node"),
                locale: CommandLocale {
                    time_zone: "UTC".into(),
                    languages: vec!["en-US".into()],
                },
            },
            operation: "crash".into(),
            invocation_id: "crash".into(),
            args: serde_json::json!({}),
            cwd: std::env::temp_dir().to_string_lossy().into_owned(),
            env: BTreeMap::new(),
            edits: None,
            json: None,
        };
        // The service may die before the call is even open.
        let error = match resident.client().invoke(&invocation).await {
            Err(error) => error,
            Ok((mut input, mut output)) => {
                // Ending input fails too once the service is gone.
                let _ = input.end();
                loop {
                    match output.next().await {
                        Ok(Some(_)) => {}
                        Ok(None) => panic!("the crashing call completed"),
                        Err(error) => break error,
                    }
                }
            }
        };
        let failure = resident.failure(error).await;
        assert!(failure.contains("exited with exit status: 3"), "{failure}");
        assert!(failure.contains("fixture crashing on purpose"), "{failure}");
        let next = acquire(&services, &descriptor, resolver).await;
        assert_eq!(call(&next, "retain", "after").await, 0);
        registry.close().await;
    })
    .await
    .unwrap();
}

/// Callers asking at once share one start; one that gives up leaves the
/// start to the others (`native-runtime.md` § Install the selected
/// executable).
#[tokio::test]
async fn a_caller_that_gives_up_leaves_the_shared_start_to_the_others() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let registry = registry(root.path()).await;
        let services = registry.handle();
        let (descriptor, path) = fixture(root.path(), 0).await;
        let resolver = Arc::new(Local {
            path,
            calls: AtomicUsize::new(0),
            gate: Some((Notify::new(), Notify::new())),
        });
        let impatient = CancellationToken::new();
        let first = tokio::spawn({
            let services = services.clone();
            let descriptor = descriptor.clone();
            let resolver = resolver.clone();
            let impatient = impatient.clone();
            async move { services.acquire(&descriptor, resolver, &impatient).await.map(|_| ()) }
        });
        let (started, proceed) = resolver.gate.as_ref().unwrap();
        started.notified().await;
        let second = tokio::spawn({
            let services = services.clone();
            let descriptor = descriptor.clone();
            let resolver = resolver.clone();
            async move {
                services
                    .acquire(&descriptor, resolver, &CancellationToken::new())
                    .await
            }
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        impatient.cancel();
        assert!(matches!(first.await.unwrap().unwrap_err().as_ref(), RuntimeError::Cancelled));
        proceed.notify_one();
        let resident = second.await.unwrap().unwrap();
        assert!(resident.client().info().await.is_ok());
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
        registry.close().await;
    })
    .await
    .unwrap();
}

/// A start nothing waits for any longer stops, and the next caller starts
/// again.
#[tokio::test]
async fn a_start_nobody_waits_for_stops() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let registry = registry(root.path()).await;
        let services = registry.handle();
        let (descriptor, path) = fixture(root.path(), 0).await;
        let resolver = Arc::new(Local {
            path,
            calls: AtomicUsize::new(0),
            gate: Some((Notify::new(), Notify::new())),
        });
        let impatient = CancellationToken::new();
        let first = tokio::spawn({
            let services = services.clone();
            let descriptor = descriptor.clone();
            let resolver = resolver.clone();
            let impatient = impatient.clone();
            async move { services.acquire(&descriptor, resolver, &impatient).await.map(|_| ()) }
        });
        let (started, proceed) = resolver.gate.as_ref().unwrap();
        started.notified().await;
        impatient.cancel();
        assert!(first.await.unwrap().is_err());
        let second = tokio::spawn({
            let services = services.clone();
            let descriptor = descriptor.clone();
            let resolver = resolver.clone();
            async move {
                services
                    .acquire(&descriptor, resolver, &CancellationToken::new())
                    .await
            }
        });
        started.notified().await;
        proceed.notify_one();
        let resident = second.await.unwrap().unwrap();
        assert!(resident.client().info().await.is_ok());
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 2);
        registry.close().await;
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn stopping_all_services_ends_every_one_and_closing_ends_the_registry() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let registry = registry(root.path()).await;
        let services = registry.handle();
        let mut residents = Vec::new();
        let mut leases = Vec::new();
        for variant in 0..2 {
            let (descriptor, path) = fixture(root.path(), variant).await;
            leases.push(services.lease(digest(&descriptor)).await);
            let resident = acquire(&services, &descriptor, local(path)).await;
            assert_eq!(call(&resident, "retain", "held").await, 0);
            residents.push(resident);
        }
        services.stop_all().await;
        for resident in &residents {
            assert!(resident.client().info().await.is_err());
        }
        registry.close().await;
        let (descriptor, path) = fixture(root.path(), 0).await;
        assert!(services.acquire(&descriptor, local(path), &CancellationToken::new()).await.is_err());
    })
    .await
    .unwrap();
}

/// Puts `bytes` where a Cloud image rooted at `image` preinstalls the
/// executable of `descriptor` (`native-runtime.md` § Preinstalled
/// executables), runnable as the image build leaves it.
async fn preinstall(image: &Path, descriptor: &PackageDescriptor, bytes: &[u8]) {
    let directory = image.join(digest(descriptor));
    tokio::fs::create_dir_all(&directory).await.unwrap();
    let name = format!("demi-native-fixture{}", std::env::consts::EXE_SUFFIX);
    let executable = directory.join(name);
    tokio::fs::write(&executable, bytes).await.unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let runnable = std::fs::Permissions::from_mode(0o755);
        tokio::fs::set_permissions(&executable, runnable)
            .await
            .unwrap();
    }
}

/// A Cloud's first command after a wake or a reset: its image holds the
/// service's executable, so the service starts from that copy, the backend
/// is not asked for it, and nothing lands in the cache.
#[tokio::test]
async fn a_service_starts_from_the_image_copy_without_a_download() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        let (descriptor, path) = fixture(root.path(), 0).await;
        let image = root.path().join("image");
        preinstall(&image, &descriptor, &tokio::fs::read(&path).await.unwrap()).await;
        let cache = root.path().join("cache");
        let registry = ServiceRegistry::new(
            cache.clone(),
            Some(image),
            root.path().into(),
            BTreeMap::new(),
        )
        .await
        .unwrap();
        let resolver = local(path);
        let _resident = acquire(&registry.handle(), &descriptor, resolver.clone()).await;
        assert_eq!(resolver.calls.load(Ordering::SeqCst), 0);
        let mut cached = tokio::fs::read_dir(cache).await.unwrap();
        assert!(cached.next_entry().await.unwrap().is_none());
        registry.close().await;
    })
    .await
    .unwrap();
}

/// Without an image copy that matches, a service starts from a download, as
/// on a Host without an image. A copy that does not match is not run, and
/// the Host log says why; an executable the image holds no copy of, as on
/// every paired device, is downloaded without a word.
#[tokio::test]
async fn without_a_matching_image_copy_a_service_starts_from_a_download() {
    tokio::time::timeout(Duration::from_secs(60), async {
        let root = tempfile::tempdir().unwrap();
        // As `main` makes it, the log is a layer of the subscriber that
        // takes information and above: here the default on this thread,
        // where the registry's tasks run.
        let (log, layer) = host_log::open(root.path().join("log")).await.unwrap();
        let subscriber = tracing_subscriber::registry().with(layer.with_filter(LevelFilter::INFO));
        let _default = tracing::subscriber::set_default(subscriber);
        let (damaged, damaged_path) = fixture(root.path(), 0).await;
        let (absent, absent_path) = fixture(root.path(), 1).await;
        let image = root.path().join("image");
        let size = tokio::fs::metadata(&damaged_path).await.unwrap().len();
        preinstall(&image, &damaged, &vec![0; size as usize]).await;
        let registry = ServiceRegistry::new(
            root.path().join("cache"),
            Some(image.clone()),
            root.path().into(),
            BTreeMap::new(),
        )
        .await
        .unwrap();
        let services = registry.handle();
        for (descriptor, path) in [(&damaged, damaged_path), (&absent, absent_path)] {
            let resolver = local(path);
            let _resident = acquire(&services, descriptor, resolver.clone()).await;
            assert_eq!(resolver.calls.load(Ordering::SeqCst), 1);
        }
        registry.close().await;
        let query = Query {
            since: None,
            limit: 100,
            source: None,
        };
        let lines = log.reader().read(query).await.unwrap().lines;
        log.close().await;
        let damaged_directory = image.join(digest(&damaged));
        let damaged_directory = damaged_directory.to_string_lossy();
        let told: Vec<_> = lines
            .iter()
            .filter(|line| line.text.contains(&*damaged_directory))
            .collect();
        let [line] = told.as_slice() else {
            panic!("one line names the damaged copy: {lines:?}");
        };
        assert_eq!(line.source, "runner");
        assert!(
            line.text.contains("does not match its declared SHA-256"),
            "{}",
            line.text
        );
        let absent_digest = digest(&absent);
        assert!(
            !lines.iter().any(|line| line.text.contains(&absent_digest)),
            "{lines:?}"
        );
    })
    .await
    .unwrap();
}
