//! The tab list and the tab methods:
//! each runs the browser's own operation as a package call that never wakes
//! the Host and answers as the page expects. The tab list the plugin holds
//! follows the Host's notice of a change, never a method.

use std::cell::Cell;
use std::rc::Rc;

use demi_command_declarations::NativeOperation;
use demi_plugin_browser::Browser;
use demi_plugin_interface::{
    Plugin, PluginError, PluginFactory, PortFailure, PortRefusal, Reply, Request,
    testing::{TestDemi, loopback},
};
use demi_web_api_protocol::ids::{ConversationId, UserId};
use demi_web_api_protocol::panel::{CreatePanelTab, PanelChange};
use serde_json::{Map, Value, json};

/// What the test's browser answers an operation.
type Answer = Box<dyn Fn(&str, &Map<String, Value>) -> Result<Value, PortRefusal>>;

pub(crate) fn world(answer: Answer) -> (Rc<dyn Plugin>, Rc<TestDemi>) {
    let demi = TestDemi::new();
    demi.package_calls.replace(Some(Box::new(
        move |operation: &NativeOperation, args, _| {
            assert_eq!(operation.package, "demi.browser");
            answer(&operation.operation, args).map_err(PortFailure::Refused)
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
        held: false,
    };
    match plugin.call(request, demi.port()).await? {
        Reply::State { state } => Ok(state),
        reply => panic!("{reply:?}"),
    }
}

/// The state the panel read takes: the held list, one read for a panel that has browser tabs, or none.
async fn held(plugin: &Rc<dyn Plugin>, demi: &Rc<TestDemi>) -> Option<Value> {
    let request = Request::PageState {
        user: UserId::try_from("u1").unwrap(),
        conversation: Some(conversation()),
        held: true,
    };
    match plugin.call(request, demi.port()).await.unwrap() {
        Reply::State { state } => Some(state),
        Reply::Done => None,
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

// Planted defect it catches: a state read that asks the Host again with
// nothing changed, a held list a method outdated or replaced before the Host
// said the tabs changed, or a held read that asks the Host.
#[tokio::test(flavor = "local")]
async fn the_tab_list_is_read_from_the_host_once_and_held_until_the_host_says_it_changed() {
    let moved = Rc::new(Cell::new(false));
    let (plugin, demi) = world(Box::new({
        let moved = moved.clone();
        move |operation, _| {
            Ok(match operation {
                "browser.tabs" => json!({
                    "list": 6,
                    "tabs": [{
                        "id": "t1",
                        "title": if moved.get() { "Moved" } else { "" },
                        "url": "about:blank",
                        "createdBy": { "kind": "user" },
                        "loading": false,
                        "canGoBack": false,
                        "canGoForward": false,
                        "shows": 0,
                    }],
                    "truncated": false,
                }),
                _ => json!({ "tab": "t1", "url": "https://example.test/", "list": 7 }),
            })
        }
    }));
    let reads = |demi: &TestDemi| {
        demi.called
            .borrow()
            .iter()
            .filter(|call| call.operation.operation == "browser.tabs")
            .count()
    };

    // Nothing is held, and a panel without browser tabs reads no Host for it.
    assert_eq!(held(&plugin, &demi).await, None);
    assert_eq!(reads(&demi), 0);
    // A panel that has a browser tab gets the list, read once and then held.
    demi.change_panel(PanelChange::Create(CreatePanelTab {
        id: "p1".into(),
        kind: "browser".into(),
        data: Map::new(),
        index: None,
    }));
    let listed = held(&plugin, &demi).await.unwrap();
    assert_eq!(held(&plugin, &demi).await, Some(listed.clone()));
    assert_eq!(tabs(&plugin, &demi).await.unwrap(), listed);
    assert_eq!(reads(&demi), 1);

    // A method leaves the held list as it is, and nothing asks the Host.
    call(&plugin, &demi, "navigate", json!({ "tab": "t1", "url": "https://example.test/" }))
        .await
        .unwrap();
    assert_eq!(held(&plugin, &demi).await, Some(listed));
    assert_eq!(reads(&demi), 1);

    // The Host says the tabs changed: the list that follows replaces it.
    moved.set(true);
    let notice = Request::PackageMessage {
        user: UserId::try_from("u1").unwrap(),
        conversation: conversation(),
        message: json!({ "tabs": "changed" }),
    };
    assert_eq!(plugin.call(notice, demi.port()).await, Ok(Reply::Done));
    let after = held(&plugin, &demi).await.unwrap();
    assert_eq!(after["tabs"][0]["title"], json!("Moved"));
    assert_eq!(reads(&demi), 2);
}
