//! The plugin behind the loopback, with a fake Exa answering its requests.

use std::rc::Rc;

use demi_command_declarations::Node;
use demi_host_interface::RpcInvocation;
use demi_host_interface::testing::{MemoryPort, test_command_context};
use demi_plugin_interface::testing::command_line::{argv, parse, roots};
use demi_plugin_interface::testing::{TestDemi, loopback};
use demi_plugin_interface::{
    Plugin, PluginError, PluginFactory, PluginId, Reply, Request, SecretName,
};
use demi_plugin_web::{Web, WebState};
use demi_web_api_protocol::ids::UserId;
use serde_json::Value;

use crate::exa::FakeExa;

/// The web plugin behind the loopback, over `demi`, whose outbound requests
/// `exa` answers.
pub struct Plugged {
    pub plugin: Rc<dyn Plugin>,
    pub demi: Rc<TestDemi>,
    pub exa: Rc<FakeExa>,
    root: Node,
}

impl Plugged {
    pub fn new(exa: FakeExa) -> Self {
        let web = Web::new();
        let demi = TestDemi::new();
        demi.plugin.replace(PluginId::try_from("web").unwrap());
        let exa = Rc::new(exa);
        let answering = exa.clone();
        demi.web_requests
            .replace(Some(Box::new(move |request| Ok(answering.answer(request)))));
        Self {
            plugin: loopback(web.instance()),
            demi,
            exa,
            root: roots(web.manifest()).remove(0),
        }
    }

    /// Runs `demi <line>` as the backend's dispatch hands it to the plugin:
    /// its exit status, stdout and stderr.
    pub async fn run(&self, line: &[&str]) -> (u8, String, String) {
        let parsed = parse(&self.root, line, None).unwrap();
        let memory = MemoryPort::new();
        self.demi.rpc.replace(Some(memory.clone()));
        let invocation = RpcInvocation {
            // The plugin host hands the plugin its path from its own group.
            path: parsed.path[1..].to_vec(),
            argv: argv(line),
            args: parsed.values,
            json: parsed.json,
            host: "laptop".into(),
            cwd: "/workspace".into(),
            env: Default::default(),
            context: test_command_context(),
            caller: None,
            stdin: false,
            pipes: None,
            brought: None,
        };
        let request = Request::Command {
            user: user(),
            invocation: Box::new(invocation),
        };
        let text = |bytes: Vec<u8>| String::from_utf8(bytes).unwrap();
        // A failure is told as the backend's dispatcher tells it, after the
        // command's path (`commands.md` § Handle an rpc call).
        let (code, told) = match self.plugin.call(request, self.demi.port()).await {
            Ok(Reply::Exit { code }) => (code, String::new()),
            Err(PluginError::Failed { message }) => {
                (1, format!("{}: {message}\n", parsed.path.join(" ")))
            }
            other => panic!("{other:?}"),
        };
        (code, text(memory.stdout()), text(memory.stderr()) + &told)
    }

    pub async fn call(&self, method: &str, params: Value) -> Result<Value, PluginError> {
        let Value::Object(params) = params else {
            panic!("parameters are an object")
        };
        let request = Request::PageCall {
            user: user(),
            method: method.to_owned(),
            params,
            conversation: None,
        };
        match self.plugin.call(request, self.demi.port()).await? {
            Reply::Result { result } => Ok(result),
            reply => panic!("{reply:?}"),
        }
    }

    pub async fn state(&self) -> WebState {
        let request = Request::PageState {
            user: user(),
            conversation: None,
        };
        match self.plugin.call(request, self.demi.port()).await.unwrap() {
            Reply::State { state } => serde_json::from_value(state).unwrap(),
            reply => panic!("{reply:?}"),
        }
    }

    /// Signs the user in with `token`, as the plugin host's callback does.
    pub fn sign_in(&self, token: &str) {
        self.demi.access_token.replace(Some(token.to_owned()));
    }

    /// The user's saved key, as the plugin keeps it.
    pub fn saved_key(&self) -> Option<String> {
        self.demi
            .secret(&SecretName::try_from("api-key").unwrap())
            .map(|secret| secret.value)
    }
}

pub fn user() -> UserId {
    UserId::try_from("user").unwrap()
}
