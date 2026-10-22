//! The simulators' panel through the JSON loopback over an in-memory Demi
//! (`crates/plugin-mobile/plugin/src/lib.rs`): which devices have a tab.

use std::{cell::RefCell, rc::Rc};

use demi_plugin_interface::{
    PluginFactory, PortFailure, PortRefusal, Request,
    testing::{TestDemi, loopback},
};
use demi_web_api_protocol::error::ErrorCode;
use demi_plugin_mobile::Mobile;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use serde_json::{Map, Value, json};

/// A Host that is stopped, as Demi refuses a look at it.
fn stopped() -> PortFailure {
    PortFailure::Refused(PortRefusal::Host {
        code: ErrorCode::HostStopped,
        status: 409,
        message: "The Cloud is stopped".into(),
    })
}

fn conversation() -> ConversationId {
    ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b").unwrap()
}

/// The conversation's Host, which lists `open` as this conversation's open
/// devices, or is stopped while `open` is none.
fn host(open: Rc<RefCell<Option<Vec<(&'static str, &'static str)>>>>) -> Rc<TestDemi> {
    let demi = TestDemi::new();
    demi.package_calls.replace(Some(Box::new(move |operation, _args: &Map<String, Value>, _kind| {
        assert_eq!(operation.operation, "mobile.list", "the sync only lists");
        let Some(open) = open.borrow().clone() else {
            return Err(stopped());
        };
        let devices: Vec<Value> = open
            .iter()
            .map(|(handle, udid)| json!({
                "name": "iPhone 17", "platform": "ios", "model": "iPhone 17", "os": "iOS 26.5",
                "state": "booted", "nativeId": udid, "open": handle, "elsewhere": false,
            }))
            .collect();
        Ok(json!({ "devices": devices, "unavailable": [] }))
    })));
    demi
}

fn tabs(demi: &TestDemi) -> Vec<String> {
    demi.panel().tabs.into_iter().map(|tab| tab.id).collect()
}

/// The panel shows the devices the conversation has open on its own Host:
/// the agent's open there adds a tab; one on an attached Host, whose
/// package tells of it too, adds none and removes none; a device that
/// closed loses its tab. Planted defect this catches: a tab announced for
/// the device another Host told of. One loopback, milliseconds.
#[tokio::test(flavor = "local")]
async fn the_panel_shows_the_devices_open_on_the_conversations_own_host() {
    let open = Rc::new(RefCell::new(Some(vec![("m1", "UDID-1")])));
    let demi = host(open.clone());
    let plugin = loopback(Mobile::new().instance());
    let told = || Request::PackageMessage {
        user: UserId::try_from("u1").unwrap(),
        conversation: conversation(),
        message: json!({ "devices": "changed" }),
    };
    plugin.call(told(), demi.port()).await.unwrap();
    assert_eq!(tabs(&demi), ["mobile-m1"]);

    // An attached Host's package says this conversation's devices changed
    // there: the conversation's own Host still has m1 alone.
    plugin.call(told(), demi.port()).await.unwrap();
    assert_eq!(tabs(&demi), ["mobile-m1"]);

    *open.borrow_mut() = Some(Vec::new());
    plugin.call(told(), demi.port()).await.unwrap();
    assert!(tabs(&demi).is_empty(), "a closed device keeps its tab");
}

/// When what the package kept on a Host ends, as when the runner restarts,
/// the panel keeps the tabs of the devices the Host lists as open and of a
/// stopped Host, and loses the tab of a device the Host no longer has.
/// Planted defect this catches: every tab removed as the package was lost.
/// One loopback, milliseconds.
#[tokio::test(flavor = "local")]
async fn a_lost_package_loses_the_tab_of_a_device_that_is_gone_alone() {
    let open = Rc::new(RefCell::new(Some(vec![("m1", "UDID-1"), ("m2", "UDID-2")])));
    let demi = host(open.clone());
    let plugin = loopback(Mobile::new().instance());
    let lost = || Request::PackageLost {
        user: UserId::try_from("u1").unwrap(),
        conversation: conversation(),
    };
    plugin
        .call(
            Request::PackageMessage {
                user: UserId::try_from("u1").unwrap(),
                conversation: conversation(),
                message: json!({ "devices": "changed" }),
            },
            demi.port(),
        )
        .await
        .unwrap();
    assert_eq!(tabs(&demi), ["mobile-m1", "mobile-m2"]);
    // The service started again and took both devices back.
    plugin.call(lost(), demi.port()).await.unwrap();
    assert_eq!(tabs(&demi), ["mobile-m1", "mobile-m2"], "a restart took a tab");
    // A stopped Host lists nothing, and its devices keep their tabs.
    *open.borrow_mut() = None;
    plugin.call(lost(), demi.port()).await.unwrap();
    assert_eq!(tabs(&demi), ["mobile-m1", "mobile-m2"], "a stopped Host took a tab");
    // The Host started again, and one device no longer exists.
    *open.borrow_mut() = Some(vec![("m1", "UDID-1")]);
    plugin.call(lost(), demi.port()).await.unwrap();
    assert_eq!(tabs(&demi), ["mobile-m1"], "a device that is gone kept its tab");
}
