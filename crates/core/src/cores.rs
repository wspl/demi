//! The way into the cores of this process from any thread: the server's
//! shard threads, each running the cores of its users, or a program's one
//! core. A service every core shares, such as the Claude Code machine on the
//! master's core, reaches a core through it.

use std::rc::Rc;

use demi_web_api_protocol::ids::UserId;
use futures_util::future::{BoxFuture, LocalBoxFuture};
use tokio::sync::oneshot;
use tokio_util::sync::CancellationToken;

use super::Shard;

/// Work sent to a core: it runs there, on the core's thread, to its end.
pub type CoreWork = Box<dyn FnOnce(Rc<Shard>, CancellationToken) -> LocalBoxFuture<'static, ()> + Send>;

/// Why a call produced no answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ShardUnavailable {
    /// The backend is shutting down and starts no new work.
    #[error("the backend is shutting down")]
    Closing,
    /// The call panicked; its shard goes on serving.
    #[error("the shard call failed")]
    Failed,
}

/// The cores of this process.
pub trait Cores: Send + Sync {
    /// Runs `work` on the core of `user` and answers once it ended. The work
    /// runs to completion even when the answer is dropped; dropping it
    /// cancels the token `work` receives.
    fn run(&self, user: &UserId, work: CoreWork) -> BoxFuture<'static, Result<(), ShardUnavailable>>;
}

impl dyn Cores + '_ {
    /// Runs `work` on the core of `user` and answers its result, as
    /// [`Cores::run`] runs it.
    pub async fn call<F, Fut, T>(&self, user: &UserId, work: F) -> Result<T, ShardUnavailable>
    where
        F: FnOnce(Rc<Shard>, CancellationToken) -> Fut + Send + 'static,
        Fut: Future<Output = T> + 'static,
        T: Send + 'static,
    {
        let (answer, answered) = oneshot::channel();
        self.run(
            user,
            Box::new(move |core, cancel| {
                Box::pin(async move {
                    let _ = answer.send(work(core, cancel).await);
                })
            }),
        )
        .await?;
        answered.await.map_err(|_| ShardUnavailable::Failed)
    }
}
