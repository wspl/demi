//! A plugin lives while the user has it on (`crates/backend-plugins/src/lib.rs`):
//! turning it off drops its instance, which ends every task the instance
//! started, and a request after it is on again reaches a new instance. Turning
//! one plugin off leaves every other plugin's instance as it was.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use demi_backend_plugins::{PageCallError, Registry};
use demi_plugin_interface::{
    Manifest, Method, Page, Plugin, PluginError, PluginFactory, PluginId, PluginPort, Reply,
    Request, Scope,
};
use futures_util::future::LocalBoxFuture;
use serde_json::{Map, Value, json};
use tokio::sync::oneshot;
use tokio_util::task::AbortOnDropHandle;

use crate::support::{Backend, call};

/// A plugin whose user method `start` starts a task that runs until the
/// instance is dropped, holding the sender the test put in `task`.
struct Worker {
    manifest: Manifest,
    task: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    /// How many instances it made.
    made: Arc<AtomicUsize>,
}

impl Worker {
    fn new(id: &str) -> Self {
        let mut manifest = Manifest::new(PluginId::try_from(id).unwrap(), id, "A worker.");
        manifest.page = Some(
            Page::new(format!("@demicodes/plugin-{id}"))
                .method(Method::new::<Map<String, Value>, Value>("start", Scope::User)),
        );
        Self {
            manifest,
            task: Arc::default(),
            made: Arc::default(),
        }
    }

    /// The end of the task the next `start` starts: an error once the task
    /// is gone.
    fn next_task(&self) -> oneshot::Receiver<()> {
        let (sender, receiver) = oneshot::channel();
        self.task.lock().unwrap().replace(sender);
        receiver
    }
}

impl PluginFactory for Worker {
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }

    fn instance(&self) -> Rc<dyn Plugin> {
        self.made.fetch_add(1, Ordering::SeqCst);
        Rc::new(Working {
            task: self.task.clone(),
            tasks: RefCell::default(),
        })
    }
}

struct Working {
    task: Arc<Mutex<Option<oneshot::Sender<()>>>>,
    /// Its tasks, which end with it, as the contract asks of an instance.
    tasks: RefCell<Vec<AbortOnDropHandle<()>>>,
}

impl Plugin for Working {
    fn call(&self, request: Request, _: PluginPort) -> LocalBoxFuture<'_, Result<Reply, PluginError>> {
        Box::pin(async move {
            let Request::PageCall { .. } = request else {
                unreachable!("the worker declares only `start`")
            };
            let held = self.task.lock().unwrap().take();
            let task = tokio::task::spawn_local(async move {
                let _held = held;
                std::future::pending::<()>().await;
            });
            self.tasks.borrow_mut().push(AbortOnDropHandle::new(task));
            Ok(Reply::Result { result: Value::Null })
        })
    }
}

/// Planted defect this catches: one instance per plugin for the shard's
/// whole life, whose task runs on after the switch.
#[tokio::test(flavor = "local")]
async fn turning_a_plugin_off_ends_its_instance_and_its_tasks_and_leaves_another_plugin_alone() {
    let (one, two) = (Worker::new("one"), Worker::new("two"));
    let (one_task, two_task) = (one.next_task(), two.next_task());
    let (one_made, two_made) = (one.made.clone(), two.made.clone());
    let one_next = one.task.clone();
    let backend = Backend::start(
        Registry::new(vec![Box::new(one), Box::new(two)], |_| true).unwrap(),
        Vec::new(),
    )
    .await;
    let user = backend.user("u1").await;
    let plugins = backend.plugins(&user);
    call(&plugins, "one", "start", json!({})).await.unwrap();
    call(&plugins, "two", "start", json!({})).await.unwrap();

    assert!(plugins.switch("one", false).await.unwrap().is_some());
    // The deadline only guards against a hang.
    let ended = tokio::time::timeout(Duration::from_secs(5), one_task).await;
    assert!(matches!(ended, Ok(Err(_))), "the task of the plugin turned off still runs");
    assert!(matches!(
        call(&plugins, "one", "start", json!({})).await,
        Err(PageCallError::Disabled(_))
    ));
    let mut two_task = two_task;
    assert_eq!(two_task.try_recv(), Err(oneshot::error::TryRecvError::Empty));

    // On again, a request reaches a new instance, whose task runs.
    assert!(plugins.switch("one", true).await.unwrap().is_some());
    let (sender, mut again) = oneshot::channel();
    one_next.lock().unwrap().replace(sender);
    call(&plugins, "one", "start", json!({})).await.unwrap();
    assert_eq!(again.try_recv(), Err(oneshot::error::TryRecvError::Empty));
    assert_eq!(one_made.load(Ordering::SeqCst), 2);
    assert_eq!(two_made.load(Ordering::SeqCst), 1);
}
