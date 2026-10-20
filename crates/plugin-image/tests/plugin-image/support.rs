//! The plugin behind the loopback, over an in-memory Demi.

use std::collections::BTreeMap;
use std::rc::Rc;

use demi_command_declarations::Node;
use demi_host_interface::RpcInvocation;
use demi_host_interface::testing::{MemoryPort, test_command_context};
use demi_plugin_interface::testing::command_line::{argv, parse, roots};
use demi_plugin_interface::testing::{TestDemi, loopback};
use demi_plugin_interface::{
    ImageAsk, MadeImage, Plugin, PluginError, PluginFactory, PluginId, PortRefusal, Reply, Request,
};
use demi_plugin_image::Image;
use demi_web_api_protocol::ids::UserId;

/// Runs `test` on a thread of its own with a local task set, as a shard
/// runs a plugin.
pub fn local<T>(test: impl Future<Output = T>) -> T {
    tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(tokio::task::LocalSet::new().run_until(test))
}

/// The image plugin behind the loopback, over `demi`.
pub struct Plugged {
    pub plugin: Rc<dyn Plugin>,
    pub demi: Rc<TestDemi>,
    root: Node,
}

impl Plugged {
    pub fn new() -> Self {
        let image = Image::new();
        let demi = TestDemi::new();
        demi.plugin.replace(PluginId::try_from("image").unwrap());
        demi.job_directory.replace("/workspace".to_owned());
        Self {
            plugin: loopback(image.instance()),
            demi,
            root: roots(image.manifest()).remove(0),
        }
    }

    /// How the Providers service answers each image asked for.
    pub fn answer(&self, answer: impl Fn(&ImageAsk) -> Result<MadeImage, PortRefusal> + 'static) {
        self.demi.image_requests.replace(Some(Box::new(answer)));
    }

    /// The files of the conversation's Host by path; none while it is not
    /// running.
    pub fn host_files(&self, files: Option<Vec<(&str, Vec<u8>)>>) {
        let files = files.map(|files| {
            files
                .into_iter()
                .map(|(path, bytes)| (path.to_owned(), bytes))
                .collect::<BTreeMap<_, _>>()
        });
        self.demi.host_files.replace(files);
    }

    /// Runs `demi <line>` from `/workspace` as the backend's dispatch hands
    /// it to the plugin: its exit status, stdout and stderr.
    pub async fn run(&self, line: &[&str]) -> (u8, Vec<u8>, String) {
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
            user: UserId::try_from("user").unwrap(),
            invocation: Box::new(invocation),
        };
        // A failure is told as the backend's dispatcher tells it, after the
        // command's path (`crates/host-interface/src/rpc.rs`).
        let (code, told) = match self.plugin.call(request, self.demi.port()).await {
            Ok(Reply::Exit { code }) => (code, String::new()),
            Err(PluginError::Failed { message }) => (1, format!("{}: {message}\n", parsed.path.join(" "))),
            other => panic!("{other:?}"),
        };
        let stderr = String::from_utf8(memory.stderr()).unwrap() + &told;
        (code, memory.stdout(), stderr)
    }
}
