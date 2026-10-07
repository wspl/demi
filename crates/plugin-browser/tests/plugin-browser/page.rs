//! The tab list and the tab methods (`live-view.md` § The tab methods):
//! each runs the browser's own operation as a package call that never wakes
//! the Host, answers as the page expects, and each method marks the tab
//! list changed.

use std::rc::Rc;

use demi_command_declarations::NativeOperation;
use demi_command_package_browser_protocol::release::{ARTIFACT, BrowserRelease};
use demi_plugin_browser::Browser;
use demi_plugin_interface::{
    CallKind, Plugin, PluginError, PluginFactory, PortRefusal, Reply, Request,
    testing::{TestDemi, loopback},
};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use serde_json::{Map, Value, json};

/// What the test's browser answers an operation.
type Answer = Box<dyn Fn(&str, &Map<String, Value>) -> Result<Value, PortRefusal>>;

pub(crate) fn world(answer: Answer) -> (Rc<dyn Plugin>, Rc<TestDemi>) {
    let demi = TestDemi::new();
    demi.package_calls.replace(Some(Box::new(
        move |operation: &NativeOperation, args, _| {
            assert_eq!(operation.package, "demi.browser");
            answer(&operation.operation, args)
        },
    )));
    (loopback(Browser::new().instance()), demi)
}

pub(crate) fn conversation() -> ConversationId {
    ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b").unwrap()
}

/// The plugin's conversation state, the tab list.
async fn tabs(plugin: &Rc<dyn Plugin>, demi: &Rc<TestDemi>) -> Result<Value, PluginError> {
    let request = Request::PageState {
        user: UserId::try_from("u1").unwrap(),
        conversation: Some(conversation()),
    };
    match plugin.call(request, demi.port()).await? {
        Reply::State { state } => Ok(state),
        reply => panic!("{reply:?}"),
    }
}

pub(crate) async fn call(
    plugin: &Rc<dyn Plugin>,
    demi: &Rc<TestDemi>,
    method: &str,
    params: Value,
) -> Result<Value, PluginError> {
    let Value::Object(params) = params else {
        unreachable!("the test passes an object")
    };
    let request = Request::PageCall {
        user: UserId::try_from("u1").unwrap(),
        method: method.into(),
        params,
        conversation: Some(conversation()),
    };
    match plugin.call(request, demi.port()).await? {
        Reply::Result { result } => Ok(result),
        reply => panic!("{reply:?}"),
    }
}

pub(crate) fn stopped() -> PortRefusal {
    PortRefusal::Host {
        code: ErrorCode::HostStopped,
        status: 409,
        message: "The Cloud is stopped".into(),
    }
}

fn tab_not_found() -> PortRefusal {
    PortRefusal::Operation {
        stderr: json!({ "error": { "code": "tab_not_found", "message": "No tab t9" } }).to_string(),
    }
}

#[tokio::test(flavor = "local")]
async fn each_method_runs_its_operation_as_a_package_call_that_never_wakes_the_host() {
    let (plugin, demi) = world(Box::new(|operation, _| {
        Ok(match operation {
            "browser.tabs" => json!({
                "list": 6,
                "tabs": [{ "id": "t1", "title": "", "url": "about:blank", "createdBy": { "kind": "user" }, "loading": false, "canGoBack": true, "canGoForward": false, "shows": 0 }],
                "truncated": false,
            }),
            "browser.goto" | "browser.back" | "browser.reload" => {
                json!({ "tab": "t1", "url": "https://example.test/", "list": 7 })
            }
            _ => json!({}),
        })
    }));

    let listed = tabs(&plugin, &demi).await.unwrap();
    assert_eq!(listed["tabs"][0]["id"], json!("t1"));
    assert_eq!(listed["tabs"][0]["canGoBack"], json!(true));
    // A navigation answers the number of the last tab list before it started.
    for (method, params, answer) in [
        (
            "navigate",
            json!({ "tab": "t1", "url": "https://example.test/" }),
            json!({ "list": 7 }),
        ),
        (
            "history",
            json!({ "tab": "t1", "action": "back" }),
            json!({ "list": 7 }),
        ),
        (
            "history",
            json!({ "tab": "t1", "action": "reload" }),
            json!({ "list": 7 }),
        ),
        ("sync", json!({}), Value::Null),
    ] {
        assert_eq!(call(&plugin, &demi, method, params).await, Ok(answer));
    }
    // Every method changed the tab list the pages show.
    assert_eq!(demi.changes(), 4);

    let calls: Vec<(String, CallKind)> = demi
        .called
        .borrow()
        .iter()
        .map(|call| (call.operation.operation.clone(), call.kind))
        .collect();
    assert_eq!(
        calls,
        [
            ("browser.tabs".to_owned(), CallKind::Looks),
            ("browser.goto".to_owned(), CallKind::Operates),
            ("browser.back".to_owned(), CallKind::Operates),
            ("browser.reload".to_owned(), CallKind::Operates),
            ("browser.tabs".to_owned(), CallKind::Looks),
        ]
    );
}

#[tokio::test(flavor = "local")]
async fn a_stopped_host_lists_no_tabs_and_a_tab_the_browser_lacks_is_refused_to_move() {
    let (plugin, demi) = world(Box::new(|operation, _| match operation {
        "browser.tabs" => Err(stopped()),
        "browser.goto" => Err(stopped()),
        _ => Err(tab_not_found()),
    }));

    // The state names the pinned browser, which the page looks for among
    // the Host's installed artifacts, even while the Host is stopped.
    let pinned = BrowserRelease::pinned().unwrap();
    assert_eq!(
        tabs(&plugin, &demi).await,
        Ok(json!({ "tabs": [], "browser": { "name": ARTIFACT, "version": pinned.version } }))
    );
    assert_eq!(
        call(&plugin, &demi, "sync", json!({})).await,
        Ok(Value::Null)
    );
    let moved = call(
        &plugin,
        &demi,
        "history",
        json!({ "tab": "t9", "action": "forward" }),
    )
    .await;
    assert!(
        matches!(&moved, Err(PluginError::Refused { reason, .. }) if reason == "tab_not_found"),
        "{moved:?}"
    );
    let moved = call(
        &plugin,
        &demi,
        "history",
        json!({ "tab": "not-a-tab", "action": "back" }),
    )
    .await;
    assert!(
        matches!(&moved, Err(PluginError::Refused { reason, .. }) if reason == "tab_not_found"),
        "{moved:?}"
    );
    // A stopped Cloud is refused with the host access's own answer, never
    // woken.
    let navigated = call(
        &plugin,
        &demi,
        "navigate",
        json!({ "tab": "t9", "url": "https://example.test/" }),
    )
    .await;
    assert_eq!(navigated, Err(PluginError::Port { refusal: stopped() }));
    // Only the sync did its work: a refused method changed no tab, so the
    // pages read nothing again.
    assert_eq!(demi.changes(), 1);
}
