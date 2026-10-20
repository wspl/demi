//! The simulators' page through the JSON loopback over an in-memory Demi
//! (`crates/plugin-mobile/plugin/src/lib.rs`): each panel control runs the operation of
//! the `demi mobile` command that does the same, with the arguments that
//! command sends.

use std::{cell::RefCell, rc::Rc};

use demi_command_package_mobile_protocol::mobile::MobileOperation;
use demi_plugin_interface::{
    CallKind, Plugin, PluginFactory, PortFailure, PortRefusal, Reply, Request,
    testing::{TestDemi, loopback},
};
use demi_web_api_protocol::error::ErrorCode;
use demi_plugin_mobile::Mobile;
use demi_web_api_protocol::ids::{ConversationId, UserId};
use serde_json::{Map, Value, json};

/// What reached the package: each operation, its arguments and its kind.
type Calls = Rc<RefCell<Vec<(String, Value, CallKind)>>>;

fn world() -> (Rc<dyn Plugin>, Rc<TestDemi>, Calls) {
    let demi = TestDemi::new();
    let calls = Calls::default();
    let seen = calls.clone();
    demi.package_calls.replace(Some(Box::new(move |operation, args: &Map<String, Value>, kind| {
        assert_eq!(operation.package, "demi.mobile");
        let name = operation.operation.clone();
        seen.borrow_mut().push((name.clone(), Value::Object(args.clone()), kind));
        Ok(match name.as_str() {
            "mobile.rotate" => json!({ "device": "m1", "orientation": "landscape-left", "width": 1568, "height": 723 }),
            "mobile.appearance" => json!({ "device": "m1", "appearance": "dark" }),
            "mobile.key" => json!({ "device": "m1", "key": args["key"] }),
            "mobile.screenshot" => json!({ "device": "m1", "path": "/scratch/mobile/m1.png", "width": 723, "height": 1568, "settled": true }),
            "mobile.record" if args["action"] == "start" => json!({ "device": "m1", "recording": true }),
            "mobile.record" => json!({ "device": "m1", "recording": false, "path": "/scratch/mobile/m1.mp4", "durationMs": 4200 }),
            "mobile.close" => json!({ "device": "m1", "shutDown": false }),
            other => panic!("the panel ran {other}"),
        })
    })));
    (loopback(Mobile::new().instance()), demi, calls)
}

async fn call(plugin: &Rc<dyn Plugin>, demi: &Rc<TestDemi>, method: &str, params: Value) -> Value {
    let Value::Object(params) = params else {
        unreachable!("the test passes an object")
    };
    let request = Request::PageCall {
        user: UserId::try_from("u1").unwrap(),
        method: method.into(),
        params,
        conversation: Some(ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b").unwrap()),
    };
    match plugin.call(request, demi.port()).await.unwrap() {
        Reply::Result { result } => result,
        reply => panic!("{reply:?}"),
    }
}

/// Each toolbar control, and the operation and arguments of its command:
/// `demi mobile rotate m1 left` is Rotate Left, `demi mobile key m1 lock`
/// is Lock, and so on.
#[tokio::test(flavor = "local")]
async fn each_panel_control_runs_its_commands_operation() {
    let (plugin, demi, calls) = world();
    let controls = [
        ("rotate", json!({ "device": "m1", "turn": "left" }), "rotate", json!({ "device": "m1", "orientation": "left" })),
        ("rotate", json!({ "device": "m1", "turn": "right" }), "rotate", json!({ "device": "m1", "orientation": "right" })),
        ("toggleAppearance", json!({ "device": "m1" }), "appearance", json!({ "device": "m1", "appearance": "toggle" })),
        ("press", json!({ "device": "m1", "button": "home" }), "key", json!({ "device": "m1", "key": "home" })),
        ("press", json!({ "device": "m1", "button": "lock" }), "key", json!({ "device": "m1", "key": "lock" })),
        ("press", json!({ "device": "m1", "button": "volume-up" }), "key", json!({ "device": "m1", "key": "volume-up" })),
        ("press", json!({ "device": "m1", "button": "volume-down" }), "key", json!({ "device": "m1", "key": "volume-down" })),
        ("screenshot", json!({ "device": "m1" }), "screenshot", json!({ "device": "m1" })),
        ("record", json!({ "device": "m1", "action": "start" }), "record", json!({ "device": "m1", "action": "start" })),
        ("record", json!({ "device": "m1", "action": "stop" }), "record", json!({ "device": "m1", "action": "stop" })),
        ("close", json!({ "device": "m1" }), "close", json!({ "device": "m1" })),
    ];
    for (method, params, operation, args) in controls {
        calls.borrow_mut().clear();
        call(&plugin, &demi, method, params).await;
        let ran = calls.borrow().clone();
        assert_eq!(ran.len(), 1, "{method} ran {ran:?}");
        let (name, sent, kind) = &ran[0];
        assert_eq!((name.as_str(), sent), (format!("mobile.{operation}").as_str(), &args), "{method}");
        // The command's own decoder takes them, as it takes the command line's.
        MobileOperation::parse(operation, sent.clone()).unwrap();
        assert_eq!(*kind, CallKind::Operates, "{method} is work on an open device");
    }
}

/// Screenshot and a stopped Record answer the file the page saves.
#[tokio::test(flavor = "local")]
async fn screenshot_and_record_answer_the_file_to_save() {
    let (plugin, demi, _calls) = world();
    let shot = call(&plugin, &demi, "screenshot", json!({ "device": "m1" })).await;
    assert_eq!(shot, json!({ "path": "/scratch/mobile/m1.png" }));
    let recorded = call(&plugin, &demi, "record", json!({ "device": "m1", "action": "stop" })).await;
    assert_eq!(recorded["path"], "/scratch/mobile/m1.mp4");
}

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

/// A device's tab carries the model its driver reads, which the toolbar
/// shows after the name, and none for a device whose driver knows none.
/// Planted defect this catches: the model dropped between the device list
/// and the tab. One loopback, milliseconds.
#[tokio::test(flavor = "local")]
async fn a_devices_tab_carries_the_model_its_driver_reads() {
    let demi = TestDemi::new();
    demi.package_calls.replace(Some(Box::new(|_operation, _args: &Map<String, Value>, _kind| {
        let device = |handle: &str, model: &str| json!({
            "name": "Work Phone", "platform": "android", "model": model, "os": "Android 16",
            "state": "booted", "nativeId": format!("AVD-{handle}"), "open": handle, "elsewhere": false,
        });
        Ok(json!({ "devices": [device("m1", "Pixel 10"), device("m2", "")], "unavailable": [] }))
    })));
    let plugin = loopback(Mobile::new().instance());
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
    let tabs = demi.panel().tabs;
    let model = |id: &str| tabs.iter().find(|tab| tab.id == id).expect("the device has a tab").data.get("model").cloned();
    assert_eq!(model("mobile-m1"), Some(json!("Pixel 10")));
    assert_eq!(model("mobile-m2"), None, "a device of no known model has none");
}

/// When what the package kept on a Host ends, the panel shows what the
/// conversation's Host has open now: an attached Host's loss leaves the
/// tabs, the Host's own stop loses them. Planted defect this catches:
/// every tab removed whichever Host's package ended. One loopback,
/// milliseconds.
#[tokio::test(flavor = "local")]
async fn a_hosts_lost_package_loses_the_tabs_of_its_devices_alone() {
    let open = Rc::new(RefCell::new(Some(vec![("m1", "UDID-1")])));
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
    plugin.call(lost(), demi.port()).await.unwrap();
    assert_eq!(tabs(&demi), ["mobile-m1"], "an attached Host's loss took the tab");
    *open.borrow_mut() = None;
    plugin.call(lost(), demi.port()).await.unwrap();
    assert!(tabs(&demi).is_empty(), "the stopped Host's device keeps its tab");
}
