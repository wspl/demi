//! A script the backend runs on a Host on its own account, as the agent's
//! shell would: a schedule's change watch, an integration Cloud's init
//! script. No page follows its job, and its output is read once it ends.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use demi_command_protocol::{CommandContext, Viewable};
use demi_host_interface::{
    CommandRecord, Ending, ExecRequest, HostError, JobCaller, Numbers, PageFeed, ShellEnvironment, WholeOutput,
};
use demi_runner_client::{ContextSource, EnvironmentOptions, RemoteHost, RemoteShellEnvironment};
use demi_shared_types::{NodeId, Sequence};
use futures_util::future::LocalBoxFuture;
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

/// How a script ended.
pub(crate) enum ScriptEnd<T> {
    /// It exited, and its output read as this.
    Read(T),
    /// It ran past its limit and was stopped.
    TimedOut,
    /// `stop` dropped it.
    Dropped,
    Failed(String),
}

/// A script's job has no page that follows it.
struct NoPage(watch::Sender<bool>);

impl PageFeed for NoPage {
    fn changed(&self, _: &Rc<RefCell<CommandRecord>>) {}

    fn watching(&self) -> watch::Receiver<bool> {
        self.0.subscribe()
    }
}

/// A script's command numbers, which no one reads.
#[derive(Default)]
struct Count(Cell<u64>);

impl Numbers for Count {
    fn next(&self, _: Sequence) -> LocalBoxFuture<'_, Result<u64, HostError>> {
        self.0.set(self.0.get() + 1);
        let next = self.0.get();
        Box::pin(async move { Ok(next) })
    }
}

/// Runs `script` in `cwd` on `host` as the agent's shell would, titled
/// `title`, at most `limit` when one is given, until `stop`; once it exits,
/// `read` reads its exit code and its whole output.
pub(crate) async fn run_script<T>(
    host: RemoteHost,
    cwd: Option<String>,
    context: CommandContext,
    script: String,
    title: &str,
    limit: Option<Duration>,
    stop: &CancellationToken,
    read: impl FnOnce(i32, Option<&WholeOutput>) -> T,
) -> ScriptEnd<T> {
    let source: ContextSource = Rc::new(move || {
        let context = context.clone();
        Box::pin(async move { Ok(context) })
    });
    let environment = RemoteShellEnvironment::new(EnvironmentOptions::new(
        host,
        source,
        Rc::new(NoPage(watch::Sender::new(false))),
        Rc::new(Count::default()),
    ));
    let request = ExecRequest {
        script,
        cwd,
        caller: JobCaller {
            node: NodeId::try_from("backend").expect("a name is not empty"),
            tool_use_id: title.to_owned(),
            turn: None,
        },
        title: title.to_owned(),
        service: true,
        viewable: Viewable {
            model: "none".into(),
            media_types: Some(Vec::new()),
        },
    };
    let cancel = CancellationToken::new();
    let command = match environment.start(request, cancel.clone()).await {
        Ok(command) => command,
        Err(error) => return ScriptEnd::Failed(error.to_string()),
    };
    let waited = async {
        match limit {
            Some(limit) => tokio::time::timeout(limit, environment.ended(&command)).await.ok(),
            None => Some(environment.ended(&command).await),
        }
    };
    let ended = tokio::select! {
        ended = waited => Some(ended),
        () = stop.cancelled() => None,
    };
    let end = match ended {
        None => ScriptEnd::Dropped,
        Some(None) => ScriptEnd::TimedOut,
        Some(Some(Err(error))) => ScriptEnd::Failed(error.to_string()),
        Some(Some(Ok(Ending::Aborted))) => ScriptEnd::Failed("the command was stopped".into()),
        Some(Some(Ok(Ending::Lost(reason)))) => ScriptEnd::Failed(reason),
        Some(Some(Ok(Ending::Exited(exit)))) => match environment.status(&command) {
            Ok(status) => ScriptEnd::Read(read(exit, status.whole.as_ref().map(|whole| &*whole.output))),
            Err(error) => ScriptEnd::Failed(error.to_string()),
        },
    };
    // A command over its time, or one dropped, is stopped, and every one let
    // go of.
    cancel.cancel();
    environment.dispose_all().await;
    end
}
