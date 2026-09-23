//! The scripted runtime the tests above providers run on.

use std::sync::Arc;

use demi_provider::{
    InferenceItem, ProviderEvent, ProviderRuntime,
    testing::{ScriptedRuntime, Turn, event, inference_request},
};
use futures_util::StreamExt;
use serde_json::json;

async fn run(runtime: &mut dyn ProviderRuntime, request: demi_provider::InferenceRequest) -> Vec<ProviderEvent> {
    runtime.run(request).collect().await
}

#[tokio::test]
async fn scripted_runs_play_in_order_and_answer_their_requests() {
    let script = ScriptedRuntime::new([
        Turn::Events(vec![event::tool_call("t1", "shell_exec", json!({ "script": "echo hi" })), event::response(3, 1)]),
        Turn::Respond(Box::new(|request| {
            let answered = request.items.iter().any(|item| matches!(item, InferenceItem::ToolResult { .. }));
            vec![event::text(if answered { "tool said hi" } else { "no result" }), event::response(5, 2)]
        })),
    ]);
    let mut runtime = script.clone();

    let first = run(&mut runtime, inference_request()).await;
    assert_eq!(first, [event::tool_call("t1", "shell_exec", json!({ "script": "echo hi" })), event::response(3, 1)]);

    let mut request = inference_request();
    request.items = Arc::new([InferenceItem::ToolResult {
        tool_use_id: "t1".into(),
        output: Vec::new(),
        is_error: false,
    }]);
    let second = run(&mut runtime, request).await;
    assert_eq!(second, [event::text("tool said hi"), event::response(5, 2)]);
    assert_eq!((script.requests().len(), script.remaining()), (2, 0));
}

#[tokio::test]
async fn a_fresh_runtime_shares_the_script_and_closing_is_counted() {
    let script = ScriptedRuntime::new([
        Turn::Events(vec![event::text("parent")]),
        Turn::Events(vec![event::text("copy")]),
    ]);
    let mut runtime = script.clone();
    let mut copy = runtime.fresh();
    assert_eq!(run(&mut runtime, inference_request()).await, [event::text("parent")]);
    assert_eq!(run(copy.as_mut(), inference_request()).await, [event::text("copy")]);
    copy.close().await;
    runtime.close().await;
    assert_eq!(script.closes(), 2);
}

#[tokio::test]
async fn cancelling_a_run_ends_it_without_a_further_event() {
    let script = ScriptedRuntime::new([Turn::pending(), Turn::Events(vec![event::text("never")])]);
    let mut runtime = script.clone();

    let request = inference_request();
    let cancel = request.cancel.clone();
    let mut run_stream = runtime.run(request);
    cancel.cancel();
    assert_eq!(run_stream.next().await, None);
    drop(run_stream);

    let request = inference_request();
    request.cancel.cancel();
    assert!(run(&mut runtime, request).await.is_empty());
}

#[tokio::test]
#[should_panic(expected = "no turn scripted for run #2")]
async fn a_run_beyond_the_script_panics() {
    let mut runtime = ScriptedRuntime::new([Turn::Events(vec![event::response(0, 0)])]);
    run(&mut runtime, inference_request()).await;
    run(&mut runtime, inference_request()).await;
}
