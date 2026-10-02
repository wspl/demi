//! `demi expose` and the expose menu's data (`expose.md` § Commands,
//! § Product surface): each request through the JSON loopback as the
//! plugin host would make it, over a Demi whose exposes, values and Hosts
//! are in memory.

use std::rc::Rc;

use demi_command_declarations::Node;
use demi_host_interface::{
    RpcInvocation,
    testing::{MemoryPort, test_command_context},
};
use demi_plugin_expose::Expose;
use demi_plugin_interface::{
    ConversationHost, HostRole, Plugin, PluginError, PluginFactory, Reply, Request,
    testing::{
        TestDemi,
        command_line::{argv, parse, roots},
        loopback,
    },
};
use demi_shared_types::Timestamp;
use demi_web_api_protocol::ids::{DeviceId, UserId};
use serde_json::{Map, Value, json};

/// The plugin's instance behind the loopback, the `demi` root a runner
/// reads its command lines with, and the Demi it runs over: a conversation
/// on `laptop`, with `ci` attached.
struct World {
    plugin: Rc<dyn Plugin>,
    root: Node,
    demi: Rc<TestDemi>,
}

fn host(name: &str, role: HostRole, online: bool) -> ConversationHost {
    ConversationHost {
        name: name.into(),
        device: DeviceId::try_from(format!("device-{name}")).unwrap(),
        role,
        online,
    }
}

impl World {
    fn new() -> Self {
        let demi = TestDemi::new();
        demi.hosts.replace(vec![
            host("laptop", HostRole::Main, true),
            host("ci", HostRole::Attached, true),
        ]);
        Self::over(demi)
    }

    /// A new instance of the plugin over `demi`, as after a backend restart.
    fn over(demi: Rc<TestDemi>) -> Self {
        let factory = Expose::new();
        let root = roots(factory.manifest()).remove(0);
        Self {
            plugin: loopback(factory.instance()),
            root,
            demi,
        }
    }

    /// Runs `demi <line>`: its exit status, stdout and stderr.
    async fn run(&self, line: &[&str]) -> (u8, String, String) {
        let parsed = parse(&self.root, line, None).unwrap();
        let memory = MemoryPort::new();
        self.demi.rpc.replace(Some(memory.clone()));
        let invocation = RpcInvocation {
            // The plugin host hands the plugin its path from its own group.
            path: parsed.path[1..].to_vec(),
            argv: argv(line),
            args: parsed.values,
            json: parsed.json,
            cwd: "/workspace".into(),
            env: Default::default(),
            context: test_command_context(),
            caller: None,
            stdin: false,
            pipes: None,
        };
        let request = Request::Command {
            user: user(),
            invocation: Box::new(invocation),
        };
        let reply = self.plugin.call(request, self.demi.port()).await.unwrap();
        let Reply::Exit { code } = reply else {
            panic!("{reply:?}")
        };
        let text = |bytes: Vec<u8>| String::from_utf8(bytes).unwrap();
        (code, text(memory.stdout()), text(memory.stderr()))
    }

    /// The plugin's state for the user's pages.
    async fn state(&self) -> Value {
        let request = Request::PageState {
            user: user(),
            conversation: None,
        };
        match self.plugin.call(request, self.demi.port()).await.unwrap() {
            Reply::State { state } => state,
            reply => panic!("{reply:?}"),
        }
    }

    /// A page's call of `method` for the expose `id`.
    async fn call(&self, method: &str, id: &str) -> Result<Value, PluginError> {
        let params = Map::from_iter([("expose".to_owned(), json!(id))]);
        let request = Request::PageCall {
            user: user(),
            method: method.into(),
            params,
            conversation: None,
        };
        match self.plugin.call(request, self.demi.port()).await? {
            Reply::Result { result } => Ok(result),
            reply => panic!("{reply:?}"),
        }
    }

    fn url(&self, number: usize) -> String {
        self.demi.live_exposes()[number].url.clone()
    }
}

fn user() -> UserId {
    UserId::try_from("u1").unwrap()
}

fn minutes(count: i64) -> Timestamp {
    Timestamp::from_millisecond(count * 60_000).unwrap()
}

#[tokio::test(flavor = "local")]
async fn exposes_are_added_on_a_conversations_hosts_and_known_by_numbers_never_given_twice() {
    let world = World::new();

    let (code, added, _) = world.run(&["expose", "add", "5173"]).await;
    assert_eq!(code, 0);
    assert_eq!(
        added,
        format!(
            "Exposed 127.0.0.1:5173 on laptop as {}\nExpires in 60 minutes (expose 1).\n",
            world.url(0)
        )
    );
    world.demi.now.set(minutes(1));
    let (code, added, _) = world
        .run(&["expose", "add", "127.0.0.1:8080", "--host", "ci"])
        .await;
    assert_eq!(code, 0);
    assert!(added.contains("on ci as"), "{added}");
    assert!(added.ends_with("(expose 2).\n"), "{added}");

    world.demi.now.set(minutes(2));
    let (_, listed, _) = world.run(&["expose", "list"]).await;
    assert_eq!(
        listed,
        format!(
            "Expose  Device  Address         Expires  URL\n\
             1       laptop  127.0.0.1:5173  58 min   {}\n\
             2       ci      127.0.0.1:8080  59 min   {}\n",
            world.url(0),
            world.url(1)
        )
    );
    let (_, listed, _) = world.run(&["expose", "list", "--json"]).await;
    let listed: Value = serde_json::from_str(&listed).unwrap();
    assert_eq!(listed["exposes"][0]["number"], json!(1));
    assert_eq!(listed["exposes"][1]["device"], json!("ci"));
    // The model never sees an expose's id, which is its URL's credential.
    assert!(listed["exposes"][0].get("id").is_none(), "{listed}");

    let (_, renewed, _) = world.run(&["expose", "renew", "1"]).await;
    assert_eq!(renewed, "Expose 1 expires in 60 minutes.\n");
    let (_, removed, _) = world.run(&["expose", "remove", "1"]).await;
    assert_eq!(removed, "Removed expose 1; its URL no longer works.\n");
    for line in [["expose", "renew", "1"], ["expose", "remove", "1"]] {
        let (code, _, refused) = world.run(&line).await;
        assert_eq!(
            (code, refused.as_str()),
            (1, format!("expose {}: no expose 1\n", line[1]).as_str())
        );
    }

    // A new instance, as after a restart, gives the next number: 1 stays
    // given.
    let restarted = World::over(world.demi.clone());
    let (_, added, _) = restarted.run(&["expose", "add", "3000"]).await;
    assert!(added.ends_with("(expose 3).\n"), "{added}");
}

#[tokio::test(flavor = "local")]
async fn an_add_the_host_access_or_the_instance_refuses_says_why_and_exits_nonzero() {
    let world = World::new();
    world.demi.hosts.borrow_mut()[1].online = false;
    for (line, reason) in [
        (
            vec!["expose", "add", "8080", "--host", "nope"],
            "expose add: host nope is not reachable from this conversation (see `demi host list`)\n",
        ),
        (
            vec!["expose", "add", "8080", "--host", "ci"],
            "expose add: the device is offline; connect it before exposing a service\n",
        ),
        (
            vec!["expose", "add", "localhost"],
            "expose add: must be host:port or a port, the port 1 to 65535\n",
        ),
    ] {
        let (code, _, refused) = world.run(&line).await;
        assert_eq!((code, refused.as_str()), (1, reason), "{line:?}");
    }
    world.demi.exposes_available.set(false);
    let (code, _, refused) = world.run(&["expose", "add", "8080"]).await;
    assert_eq!(
        (code, refused.as_str()),
        (
            1,
            "expose add: exposes are not available on this instance\n"
        )
    );
    assert_eq!(
        world.state().await,
        json!({ "available": false, "exposes": [] })
    );
}

#[tokio::test(flavor = "local")]
async fn the_page_state_shows_every_live_expose_with_its_number_and_the_menu_renews_and_removes_by_id()
 {
    let world = World::new();
    world.run(&["expose", "add", "5173"]).await;
    let expose = world.demi.live_exposes().remove(0);

    let state = world.state().await;
    assert_eq!(
        state,
        json!({
            "available": true,
            "exposes": [{
                "id": expose.id,
                "number": 1,
                "deviceId": "device-laptop",
                "deviceName": "laptop",
                "address": "127.0.0.1:5173",
                "url": expose.url,
                "expiresAt": minutes(60),
            }],
        })
    );

    world.demi.now.set(minutes(30));
    assert_eq!(
        world.call("renew", expose.id.as_str()).await,
        Ok(Value::Null)
    );
    assert_eq!(
        world.state().await["exposes"][0]["expiresAt"],
        json!(minutes(90))
    );
    assert_eq!(
        world.call("remove", expose.id.as_str()).await,
        Ok(Value::Null)
    );
    assert_eq!(world.state().await["exposes"], json!([]));
    for id in [expose.id.as_str(), "not-an-id"] {
        let refused = world.call("renew", id).await.unwrap_err();
        let PluginError::Refused { reason, .. } = refused else {
            panic!("{refused:?}")
        };
        assert_eq!(reason, "expose_not_found", "{id}");
    }
}
