//! The plugin's requests through the loopback.

use std::cell::RefCell;
use std::rc::Rc;

use demi_command_declarations::Node;
use demi_plugin_interface::testing::command_line::roots;
use demi_plugin_interface::testing::{TestDemi, loopback};
use demi_plugin_interface::{
    Plugin, PluginError, PluginFactory, PluginId, Reply, Request, WatchSubject,
};
use demi_plugin_skills::{Skills, SkillsState};
use demi_shared_types::{NodeId, TurnId};
use demi_web_api_protocol::ids::{ConversationId, UserId};
use serde_json::{Value, json};

/// The plugin of `skills` behind the loopback, over `demi`, and the `demi`
/// root a runner reads its command lines with.
pub struct Plugged {
    pub plugin: Rc<dyn Plugin>,
    pub demi: Rc<TestDemi>,
    pub root: Node,
    /// What the last project search found.
    pub project: RefCell<Option<Value>>,
}

impl Plugged {
    pub fn new(skills: &Skills) -> Self {
        let demi = TestDemi::new();
        demi.plugin.replace(PluginId::try_from("skills").unwrap());
        Self {
            plugin: loopback(skills.instance()),
            demi,
            root: roots(skills.manifest()).remove(0),
            project: RefCell::default(),
        }
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

    pub async fn state(&self) -> SkillsState {
        let request = Request::PageState {
            held: false,
            user: user(),
            conversation: None,
        };
        match self.plugin.call(request, self.demi.port()).await.unwrap() {
            Reply::State { state } => serde_json::from_value(state).unwrap(),
            reply => panic!("{reply:?}"),
        }
    }

    /// Adds `origin` and waits for its fetch to end: the call marks the
    /// state, and the fetch marks it as it starts and as it ends.
    pub async fn add(&self, origin: &str) -> String {
        let before = self.demi.changes();
        let added = self
            .call("add_source", json!({ "origin": origin }))
            .await
            .unwrap();
        self.demi.until(|demi| demi.changes() >= before + 3).await;
        added["source"].as_str().unwrap().to_owned()
    }

    /// Updates `source` and waits for its fetch to end.
    pub async fn update(&self, source: &str) {
        let before = self.demi.changes();
        self.call("update_source", json!({ "source": source }))
            .await
            .unwrap();
        self.demi.until(|demi| demi.changes() >= before + 2).await;
    }

    pub async fn enable(&self, source: &str, skill: &str) -> Result<Value, PluginError> {
        let params = json!({ "source": source, "skill": skill, "enabled": true });
        self.call("set_enabled", params).await
    }

    /// The block the plugin adds before a node's request, which knew of
    /// the blocks `seen`, with what the last [`Plugged::search`] found as
    /// the backend holds it.
    pub async fn context(&self, seen: &[&str]) -> Option<String> {
        let request = Request::Context {
            user: user(),
            conversation: conversation(),
            node: NodeId::try_from("conversation").unwrap(),
            cwd: "/home/me/app".to_owned(),
            paths: demi_host_interface::HostPaths::Posix,
            turn: TurnId::try_from("t1").unwrap(),
            seen: seen.iter().map(|text| (*text).to_owned()).collect(),
            project: self.project.borrow().clone(),
        };
        match self.plugin.call(request, self.demi.port()).await.unwrap() {
            Reply::Context { text } => text,
            reply => panic!("{reply:?}"),
        }
    }

    /// Searches the project directory `directory` as the backend asks the
    /// plugin to, keeps what it found for the next contexts, and answers
    /// what the search names to watch.
    pub async fn search(&self, directory: &str) -> Vec<WatchSubject> {
        let request = Request::ProjectSearch {
            user: user(),
            conversation: conversation(),
            directory: directory.to_owned(),
            paths: demi_host_interface::HostPaths::Posix,
        };
        match self.plugin.call(request, self.demi.port()).await.unwrap() {
            Reply::Project { found, watch } => {
                self.project.replace(Some(found));
                watch
            }
            reply => panic!("{reply:?}"),
        }
    }

    /// Gives the conversation's primary Host `files`, by absolute path, and
    /// says it runs.
    pub fn host(&self, files: &[(&str, &str)]) {
        let files = files
            .iter()
            .map(|(path, text)| ((*path).to_owned(), text.as_bytes().to_vec()))
            .collect();
        self.demi.host_files.replace(Some(files));
    }
}

fn conversation() -> ConversationId {
    ConversationId::try_from("0b6f7f3e-8f3a-4c1e-9d2b-7a1c2e3f4a5b").unwrap()
}

pub fn user() -> UserId {
    UserId::try_from("user").unwrap()
}

/// Runs `test` where the plugin can start its tasks.
pub async fn local<T>(test: impl Future<Output = T>) -> T {
    tokio::task::LocalSet::new().run_until(test).await
}
