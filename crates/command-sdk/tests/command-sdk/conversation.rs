use std::{future::Future, pin::Pin, sync::Arc, time::Duration};

use demi_command_protocol::{Completion, ConversationRequest, Invocation, Record};
use demi_command_sdk::{
    Client, ConversationContext, Handler, InvocationContext, ServiceError, serve,
};
use tokio::sync::Notify;

struct Stateless;

impl Handler for Stateless {
    type Metadata = Invocation;

    fn operations(&self) -> Vec<String> {
        vec!["noop".into()]
    }

    fn invoke(
        &self,
        _context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        Box::pin(async {
            Ok(Completion {
                exit_code: 0,
                error: None,
            })
        })
    }
}

struct Lifecycle {
    started: Arc<Notify>,
    cancelled: Arc<Notify>,
    closed: Arc<Notify>,
}

impl Handler for Lifecycle {
    type Metadata = Invocation;

    fn operations(&self) -> Vec<String> {
        vec!["noop".into()]
    }

    fn invoke(
        &self,
        context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        Stateless.invoke(context)
    }

    fn conversation(
        &self,
        context: ConversationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        let started = self.started.clone();
        let cancelled = self.cancelled.clone();
        Box::pin(async move {
            started.notify_one();
            if matches!(&context.request, ConversationRequest::Release { conversation } if conversation == "wait")
            {
                context.cancellation.cancelled().await;
                cancelled.notify_one();
                return Err(ServiceError::Cancelled);
            }
            Err(ServiceError::failed("profile cleanup failed"))
        })
    }

    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), ServiceError>> + Send>> {
        let closed = self.closed.clone();
        Box::pin(async move {
            closed.notify_one();
            Ok(())
        })
    }
}

#[tokio::test]
async fn conversation_cancellation_joins_hook_and_cleanup_failure_retires_service() {
    tokio::time::timeout(Duration::from_secs(5), async {
        let started = Arc::new(Notify::new());
        let cancelled = Arc::new(Notify::new());
        let closed = Arc::new(Notify::new());
        let (io, service) = tokio::io::duplex(4096);
        let server = tokio::spawn(serve(service, Arc::new(Lifecycle { started: started.clone(), cancelled: cancelled.clone(), closed: closed.clone() })));
        let (client, connection) = Client::connect(io).await.unwrap();
        let driver = tokio::spawn(connection);
        let (mut input, output) = client.conversation(&ConversationRequest::Release { conversation: "wait".into() }).await.unwrap();
        started.notified().await;
        input.cancel();
        cancelled.notified().await;
        drop(input);
        drop(output);
        assert_eq!(client.info().await.unwrap().operations, ["noop"]);
        let (_input, mut output) = client.conversation(&ConversationRequest::Release { conversation: "fail".into() }).await.unwrap();
        // Cleanup failure closes the service; the final failure record may race
        // connection teardown, but it must never report successful completion.
        if let Ok(Some(Record::Completion(completion))) = output.next().await {
            assert_eq!(completion.exit_code, 1);
        }
        assert!(matches!(server.await.unwrap(), Err(ServiceError::ConversationCleanup(message)) if message.contains("profile cleanup failed")));
        closed.notified().await;
        drop(_input);
        drop(output);
        drop(client);
        // A fatal service exit may reset the parent connection.
        let _ = driver.await.unwrap();
    }).await.unwrap();
}
