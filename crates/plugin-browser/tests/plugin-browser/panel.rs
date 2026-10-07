//! The `browser` kind's tabs on the backend (`live-view.md` § A browser tab
//! in the panel): a tab its user creates opens a browser tab on the address
//! the user asked for last, a tab its user closes closes it, and the
//! agent's tabs are added once and never again once their user closed them.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use demi_command_declarations::NativeOperation;
use demi_plugin_browser::Browser;
use demi_plugin_interface::{
    CallKind, PanelTabChange, Plugin, PluginFactory, PortRefusal, Reply, Request, Topic,
    testing::{TestDemi, loopback},
};
use demi_web_api_protocol::error::ErrorCode;
use demi_web_api_protocol::ids::UserId;
use demi_web_api_protocol::panel::{CreatePanelTab, PanelChange, PanelTab};
use serde_json::{Map, Value, json};

use crate::page::{call, conversation};

/// What the test's browser answers an operation, with the Demi whose panel
/// the user changes meanwhile.
type Answer = Box<dyn Fn(&TestDemi, &str, &Map<String, Value>) -> Result<Value, PortRefusal>>;

fn world(answer: Answer) -> (Rc<dyn Plugin>, Rc<TestDemi>) {
    let demi = TestDemi::new();
    let user: Weak<TestDemi> = Rc::downgrade(&demi);
    demi.package_calls.replace(Some(Box::new(
        move |operation: &NativeOperation, args, _| {
            let demi = user.upgrade().expect("the test's Demi outlives its calls");
            answer(&demi, &operation.operation, args)
        },
    )));
    (loopback(Browser::new().instance()), demi)
}

fn data(value: Value) -> Map<String, Value> {
    let Value::Object(data) = value else {
        unreachable!("the test passes an object")
    };
    data
}

/// The tab the user creates, as the page creates it.
fn created(demi: &TestDemi, id: &str, value: Value) -> PanelTab {
    let create = CreatePanelTab {
        id: id.into(),
        kind: "browser".into(),
        data: data(value),
        index: None,
    };
    demi.change_panel(PanelChange::Create(create));
    demi.panel_tab(id)
        .expect("the panel has the tab it created")
}

/// The backend telling the plugin what the user did to `tab`.
async fn told(plugin: &Rc<dyn Plugin>, demi: &Rc<TestDemi>, change: PanelTabChange, tab: PanelTab) {
    let request = Request::PanelTab {
        user: UserId::try_from("u1").unwrap(),
        conversation: conversation(),
        change,
        tab,
    };
    assert_eq!(plugin.call(request, demi.port()).await, Ok(Reply::Done));
}

/// A job of the conversation ended.
async fn job_ended(plugin: &Rc<dyn Plugin>, demi: &Rc<TestDemi>) {
    let request = Request::Topic {
        user: UserId::try_from("u1").unwrap(),
        topic: Topic::Jobs,
        conversation: Some(conversation()),
    };
    assert_eq!(plugin.call(request, demi.port()).await, Ok(Reply::Done));
}

/// Each operation the plugin ran, with its kind and the tab or URL it
/// named.
fn calls(demi: &TestDemi) -> Vec<(String, CallKind, Value)> {
    demi.called
        .borrow()
        .iter()
        .filter(|call| call.operation.operation != "browser.tabs")
        .map(|call| {
            let named = call
                .args
                .get("tab")
                .or_else(|| call.args.get("url"))
                .cloned()
                .unwrap_or(Value::Null);
            (call.operation.operation.clone(), call.kind, named)
        })
        .collect()
}

fn tab_data(demi: &TestDemi, id: &str) -> Value {
    Value::Object(demi.panel_tab(id).expect("the panel has the tab").data)
}

#[tokio::test(flavor = "local")]
async fn a_tab_its_user_created_opens_on_the_address_the_user_asked_for_last() {
    let (plugin, demi) = world(Box::new(|demi, operation, _| match operation {
        "browser.open" => {
            // The user types an address while the tab opens.
            demi.change_panel(PanelChange::Update {
                id: "a".into(),
                data: data(json!({ "url": "https://example.test/" })),
            });
            Ok(json!({ "tab": "t1", "url": "about:blank" }))
        }
        "browser.goto" => Ok(json!({ "tab": "t1", "url": "https://example.test/", "list": 3 })),
        _ => Ok(json!({})),
    }));
    let tab = created(&demi, "a", json!({ "url": "about:blank" }));
    told(&plugin, &demi, PanelTabChange::Created, tab).await;

    assert_eq!(
        tab_data(&demi, "a"),
        json!({ "url": "https://example.test/", "tab": "t1" })
    );
    assert_eq!(
        calls(&demi),
        [
            (
                "browser.open".to_owned(),
                CallKind::Starts,
                json!("about:blank")
            ),
            ("browser.goto".to_owned(), CallKind::Operates, json!("t1")),
        ]
    );
    // A tab that shows its browser tab opens nothing more.
    assert_eq!(
        call(&plugin, &demi, "bind", json!({ "panelTab": "a" })).await,
        Ok(json!({ "tab": "t1" }))
    );
    assert_eq!(calls(&demi).len(), 2);
}

#[tokio::test(flavor = "local")]
async fn a_tab_its_user_closed_while_it_opened_closes_the_browser_tab() {
    let (plugin, demi) = world(Box::new(|demi, operation, _| match operation {
        "browser.open" => {
            demi.change_panel(PanelChange::Remove { id: "a".into() });
            Ok(json!({ "tab": "t1", "url": "about:blank" }))
        }
        _ => Ok(json!({})),
    }));
    let tab = created(&demi, "a", json!({ "url": "about:blank" }));
    told(&plugin, &demi, PanelTabChange::Created, tab).await;

    assert_eq!(demi.panel().tabs, []);
    assert_eq!(
        calls(&demi),
        [
            (
                "browser.open".to_owned(),
                CallKind::Starts,
                json!("about:blank")
            ),
            ("browser.close".to_owned(), CallKind::Operates, json!("t1")),
        ]
    );
}

#[tokio::test(flavor = "local")]
async fn a_tab_that_could_not_open_says_why_and_opens_when_its_user_retries() {
    let opens = Rc::new(Cell::new(0));
    let counted = opens.clone();
    let (plugin, demi) = world(Box::new(move |_, operation, _| match operation {
        "browser.open" => {
            counted.set(counted.get() + 1);
            if counted.get() == 1 {
                return Err(PortRefusal::Host {
                    code: ErrorCode::DeviceOffline,
                    status: 409,
                    message: "The device is offline".into(),
                });
            }
            Ok(json!({ "tab": "t2", "url": "about:blank" }))
        }
        _ => Ok(json!({})),
    }));
    let tab = created(&demi, "a", json!({ "url": "about:blank" }));
    told(&plugin, &demi, PanelTabChange::Created, tab).await;
    assert_eq!(
        tab_data(&demi, "a"),
        json!({
            "url": "about:blank",
            "failure": { "code": "device_offline", "message": "The device is offline" },
        })
    );

    assert_eq!(
        call(&plugin, &demi, "bind", json!({ "panelTab": "a" })).await,
        Ok(json!({ "tab": "t2" }))
    );
    assert_eq!(
        tab_data(&demi, "a"),
        json!({ "url": "about:blank", "tab": "t2" })
    );
    assert_eq!(opens.get(), 2);
}

#[tokio::test(flavor = "local")]
async fn a_tab_its_user_removed_closes_its_browser_tab() {
    let (plugin, demi) = world(Box::new(|_, _, _| Ok(json!({}))));
    let bound = created(&demi, "a", json!({ "url": "about:blank", "tab": "t3" }));
    let unbound = created(&demi, "b", json!({ "url": "about:blank" }));
    for tab in [bound, unbound] {
        demi.change_panel(PanelChange::Remove { id: tab.id.clone() });
        told(&plugin, &demi, PanelTabChange::Removed, tab).await;
    }
    assert_eq!(
        calls(&demi),
        [("browser.close".to_owned(), CallKind::Operates, json!("t3"))]
    );
}

#[tokio::test(flavor = "local")]
async fn the_agents_tabs_are_added_once_and_never_again_once_their_user_closed_them() {
    let listed: Rc<RefCell<Value>> = Rc::new(RefCell::new(json!([])));
    let browser = listed.clone();
    let (plugin, demi) = world(Box::new(move |_, operation, _| match operation {
        "browser.tabs" => Ok(json!({ "list": 1, "tabs": browser.borrow().clone(), "truncated": false })),
        "browser.open" => Ok(json!({ "tab": "t4", "url": "https://user.test/" })),
        _ => Ok(json!({})),
    }));
    created(
        &demi,
        "a",
        json!({ "url": "https://user.test/", "tab": "t1" }),
    );
    let tab = |id: &str, by: Value| json!({ "id": id, "title": id, "url": format!("https://{id}.test/"), "createdBy": by, "loading": false, "canGoBack": false, "canGoForward": false, "shows": 0 });
    listed.replace(json!([
        tab("t1", json!({ "kind": "user" })),
        tab("t2", json!({ "kind": "agent", "number": 1 })),
        tab("t3", json!({ "kind": "page", "opener": "t2" })),
        // A tab the user opened that no panel tab shows is no tab of the agent's.
        tab("t5", json!({ "kind": "user" })),
    ]));

    job_ended(&plugin, &demi).await;
    let ids = |demi: &TestDemi| -> Vec<String> {
        demi.panel().tabs.into_iter().map(|tab| tab.id).collect()
    };
    assert_eq!(ids(&demi), ["a", "browser-t2", "browser-t3"]);
    assert_eq!(
        tab_data(&demi, "browser-t2"),
        json!({ "url": "https://t2.test/", "tab": "t2", "title": "t2" })
    );
    // Reading the list again adds nothing, and a tab the user closed stays closed.
    let revision = demi.panel().revision;
    job_ended(&plugin, &demi).await;
    assert_eq!(demi.panel().revision, revision);
    demi.change_panel(PanelChange::Remove {
        id: "browser-t2".into(),
    });
    job_ended(&plugin, &demi).await;
    assert_eq!(ids(&demi), ["a", "browser-t3"]);

    // The browser lost the user's tab and the page's: both say so, and stay.
    listed.replace(json!([tab("t2", json!({ "kind": "agent", "number": 1 }))]));
    job_ended(&plugin, &demi).await;
    assert_eq!(ids(&demi), ["a", "browser-t3"]);
    assert_eq!(tab_data(&demi, "a")["closed"], json!(true));
    assert_eq!(tab_data(&demi, "browser-t3")["closed"], json!(true));
    // Shown, it opens a new browser tab on the saved address.
    assert_eq!(
        call(&plugin, &demi, "bind", json!({ "panelTab": "a" })).await,
        Ok(json!({ "tab": "t4" }))
    );
    assert_eq!(
        tab_data(&demi, "a"),
        json!({ "url": "https://user.test/", "tab": "t4" })
    );
}

/// A tab closed on purpose leaves the panel, as a browser's closed tab does;
/// a tab lost with the browser stays with its address and title, and opens
/// again on its address when its content asks, once shown
/// (`live-view.md` § A browser tab in the panel).
#[tokio::test(flavor = "local")]
async fn a_tab_closed_on_purpose_leaves_the_panel_and_one_lost_with_the_browser_opens_again() {
    let listed: Rc<RefCell<Value>> = Rc::new(RefCell::new(json!({ "tabs": [], "closed": [] })));
    let browser = listed.clone();
    let (plugin, demi) = world(Box::new(move |_, operation, _| match operation {
        "browser.tabs" => {
            let listed = browser.borrow();
            Ok(json!({ "list": 1, "tabs": listed["tabs"], "truncated": false, "closed": listed["closed"] }))
        }
        "browser.open" => Ok(json!({ "tab": "t9", "url": "https://lost.test/" })),
        _ => Ok(json!({})),
    }));
    created(&demi, "closed", json!({ "url": "https://closed.test/", "tab": "t1" }));
    created(
        &demi,
        "lost",
        json!({ "url": "https://lost.test/", "tab": "t2", "title": "Lost" }),
    );

    // The agent closed t1; the browser that had t2 ended.
    listed.replace(json!({ "tabs": [], "closed": ["t1"] }));
    job_ended(&plugin, &demi).await;
    let ids: Vec<String> = demi.panel().tabs.into_iter().map(|tab| tab.id).collect();
    assert_eq!(ids, ["lost"]);
    assert_eq!(
        tab_data(&demi, "lost"),
        json!({ "url": "https://lost.test/", "tab": "t2", "title": "Lost", "closed": true })
    );
    // Shown, it opens a new browser tab on its address, which wakes a
    // stopped Cloud as any new tab does.
    assert_eq!(
        call(&plugin, &demi, "bind", json!({ "panelTab": "lost" })).await,
        Ok(json!({ "tab": "t9" }))
    );
    assert_eq!(
        calls(&demi),
        [(
            "browser.open".to_owned(),
            CallKind::Starts,
            json!("https://lost.test/")
        )]
    );
    assert_eq!(
        tab_data(&demi, "lost"),
        json!({ "url": "https://lost.test/", "tab": "t9", "title": "Lost" })
    );
}

#[tokio::test(flavor = "local")]
async fn a_tab_the_agent_showed_carries_the_count_into_its_panel_tab() {
    let listed: Rc<RefCell<Value>> = Rc::new(RefCell::new(json!([])));
    let browser = listed.clone();
    let (plugin, demi) = world(Box::new(move |_, operation, _| match operation {
        "browser.tabs" => Ok(json!({ "list": 1, "tabs": browser.borrow().clone(), "truncated": false })),
        _ => Ok(json!({})),
    }));
    created(&demi, "a", json!({ "url": "https://t1.test/", "tab": "t1" }));
    let tab = |id: &str, by: Value, shows: u64| json!({ "id": id, "title": id, "url": format!("https://{id}.test/"), "createdBy": by, "loading": false, "canGoBack": false, "canGoForward": false, "shows": shows });
    listed.replace(json!([
        // A tab the user created is shown as the agent's are.
        tab("t1", json!({ "kind": "user" }), 2),
        tab("t2", json!({ "kind": "agent", "number": 1 }), 1),
        tab("t3", json!({ "kind": "agent", "number": 1 }), 0),
    ]));

    job_ended(&plugin, &demi).await;
    assert_eq!(tab_data(&demi, "a")["shows"], json!(2));
    assert_eq!(
        tab_data(&demi, "browser-t2"),
        json!({ "url": "https://t2.test/", "tab": "t2", "title": "t2", "shows": 1 })
    );
    // A tab never shown has no count.
    assert_eq!(tab_data(&demi, "browser-t3").get("shows"), None);
    // The same counts change nothing; a higher one is written again.
    let revision = demi.panel().revision;
    job_ended(&plugin, &demi).await;
    assert_eq!(demi.panel().revision, revision);
    listed.replace(json!([
        tab("t1", json!({ "kind": "user" }), 2),
        tab("t2", json!({ "kind": "agent", "number": 1 }), 1),
        tab("t3", json!({ "kind": "agent", "number": 1 }), 1),
    ]));
    job_ended(&plugin, &demi).await;
    assert_eq!(tab_data(&demi, "browser-t3")["shows"], json!(1));
}

/// A tab a page opened stands right after its opener's, behind the ones the
/// same opener opened before it, whether its page opened them or the user's
/// Open Link in New Tab did, as Chrome places the tabs a link opens; the
/// panel keeps which tab opened which, so the order holds across reads and
/// reloads. The agent's tabs, and one whose opener the panel does not show,
/// go after the others (`live-view.md` § A browser tab in the panel).
#[tokio::test(flavor = "local")]
async fn a_tab_a_page_opened_stands_beside_its_opener() {
    let listed: Rc<RefCell<Value>> = Rc::new(RefCell::new(json!([])));
    let browser = listed.clone();
    let (plugin, demi) = world(Box::new(move |_, operation, _| match operation {
        "browser.tabs" => Ok(json!({ "list": 1, "tabs": browser.borrow().clone(), "truncated": false })),
        _ => Ok(json!({})),
    }));
    created(&demi, "a", json!({ "url": "https://a.test/", "tab": "t1" }));
    created(&demi, "b", json!({ "url": "https://b.test/", "tab": "t2" }));
    // Open Link in New Tab on `a`, as the page places it: right after `a`.
    demi.change_panel(PanelChange::Create(CreatePanelTab {
        id: "link".into(),
        kind: "browser".into(),
        data: data(json!({ "url": "https://link.test/", "openedBy": "a" })),
        index: Some(1),
    }));
    let tab = |id: &str, by: Value| json!({ "id": id, "title": id, "url": format!("https://{id}.test/"), "createdBy": by, "loading": false, "canGoBack": false, "canGoForward": false, "shows": 0 });
    let by_page = |opener: &str| json!({ "kind": "page", "opener": opener });
    let ids = |demi: &TestDemi| -> Vec<String> {
        demi.panel().tabs.into_iter().map(|tab| tab.id).collect()
    };
    let mut tabs = vec![
        tab("t1", json!({ "kind": "user" })),
        tab("t2", json!({ "kind": "user" })),
        tab("t3", by_page("t1")),
        tab("t4", by_page("t1")),
        tab("t5", by_page("t2")),
        tab("t6", json!({ "kind": "agent", "number": 1 })),
        // Its opener is gone from the browser, so the panel shows no tab for it.
        tab("t7", by_page("t9")),
    ];
    listed.replace(Value::Array(tabs.clone()));

    job_ended(&plugin, &demi).await;
    assert_eq!(
        ids(&demi),
        ["a", "link", "browser-t3", "browser-t4", "b", "browser-t5", "browser-t6", "browser-t7"]
    );
    assert_eq!(tab_data(&demi, "browser-t3")["openedBy"], json!("a"));
    assert_eq!(tab_data(&demi, "browser-t7").get("openedBy"), None);
    // The opener's next tab joins the ones it opened, after them: the panel
    // remembers which those are, not the page or the plugin.
    tabs.push(tab("t8", by_page("t1")));
    listed.replace(Value::Array(tabs));
    job_ended(&plugin, &demi).await;
    assert_eq!(
        ids(&demi),
        ["a", "link", "browser-t3", "browser-t4", "browser-t8", "b", "browser-t5", "browser-t6", "browser-t7"]
    );
}
