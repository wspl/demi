//! `demi mobile`'s command contract against a scripted driver
//! (`crates/command-package-mobile/src/operations.rs`,
//! `crates/command-package-mobile/src/devices.rs`): what each command prints
//! and what
//! reaches the device, through the service as the runner calls it.

mod commands;
mod ios;
mod live;
mod lifetime;
mod android;
mod frames;

use std::{collections::BTreeMap, sync::Arc};

use demi_command_package_mobile_protocol::mobile::{MobilePlatform, PowerState};
use demi_command_protocol::{
    CommandCaller, CommandContext, CommandLocale, Completion, ConversationRequest, Invocation,
    Record,
};
use demi_command_sdk::{Client, serve, testing::answer_numbers};
use demi_mobile::{
    DemiMobile,
    driver::{Absent, Driver, RawElement, Rect, Simulator},
    testing::FakeDriver,
};
use serde_json::Value;
use tokio_util::task::AbortOnDropHandle;

pub const CHAT: &str = "chat-a";
pub const OTHER: &str = "chat-b";

/// The Host's simulators in these tests: one the user booted, the rest shut down.
pub fn simulators() -> Vec<Simulator> {
    vec![
        FakeDriver::simulator("UDID-BOOTED", "iPhone 17 Pro", PowerState::Booted),
        FakeDriver::simulator("UDID-1", "iPhone 17", PowerState::ShutDown),
        FakeDriver::simulator("UDID-2", "iPhone 16e", PowerState::ShutDown),
        FakeDriver::simulator("UDID-3", "iPhone Air", PowerState::ShutDown),
        FakeDriver::simulator("UDID-4", "iPad Air", PowerState::ShutDown),
    ]
}

/// The service with a scripted iOS driver and, as on a Mac without the
/// Android SDK, no Android.
pub struct Mobile {
    pub driver: Arc<FakeDriver>,
    pub service: Arc<DemiMobile>,
    client: Client,
    _numbers: AbortOnDropHandle<Result<(), demi_command_sdk::ServiceError>>,
    _connection: AbortOnDropHandle<()>,
    _server: AbortOnDropHandle<Result<(), demi_command_sdk::ServiceError>>,
    pub root: tempfile::TempDir,
    _home: Option<tempfile::TempDir>,
}

/// One command's answer.
#[derive(Debug)]
pub struct Answer {
    pub exit_code: u8,
    pub stdout: Vec<u8>,
    pub stderr: String,
}

impl Answer {
    pub fn json(&self) -> Value {
        assert_eq!(self.exit_code, 0, "the command failed: {}", self.stderr);
        serde_json::from_slice(&self.stdout).expect("the result is JSON")
    }

    /// The failure document's code and message.
    pub fn failure(&self) -> (String, String) {
        assert_ne!(self.exit_code, 0, "the command succeeded");
        let document: Value = serde_json::from_str(&self.stderr).expect("the failure is JSON");
        (
            document["error"]["code"].as_str().unwrap().to_owned(),
            document["error"]["message"].as_str().unwrap().to_owned(),
        )
    }
}

impl Mobile {
    pub async fn start() -> Self {
        let home = tempfile::tempdir().unwrap();
        let mobile = home.path().join("mobile");
        Self::start_on(FakeDriver::ios(simulators()), &mobile, Some(home)).await
    }

    /// The service over `driver`, keeping what it boots in `mobile`, as a
    /// program started again on the same Host.
    pub async fn start_on(driver: Arc<FakeDriver>, mobile: &std::path::Path, home: Option<tempfile::TempDir>) -> Self {
        Self::start_over(driver.clone(), driver, mobile, home).await
    }

    /// The service over the iOS driver reaching its helper through
    /// `launch`, as on a Mac.
    pub async fn start_ios(launch: Arc<dyn demi_mobile::ios::Launch>) -> Self {
        let home = tempfile::tempdir().unwrap();
        let mobile = home.path().join("mobile");
        let ios = Arc::new(demi_mobile::ios::Ios::with_launch(launch));
        Self::start_over(ios, FakeDriver::ios(Vec::new()), &mobile, Some(home)).await
    }

    /// The service over a scripted iOS driver with `simulators` and the
    /// Android driver over `sdk`, as on a Mac with Xcode and the Android SDK.
    pub async fn start_android(simulators: Vec<Simulator>, sdk: Arc<dyn demi_mobile::android::Sdk>) -> Self {
        let home = tempfile::tempdir().unwrap();
        let mobile = home.path().join("mobile");
        let ios = FakeDriver::ios(simulators);
        let drivers: Vec<Arc<dyn Driver>> = vec![ios.clone(), Arc::new(demi_mobile::android::Android::with_sdk(sdk, Arc::new(demi_mobile::testing::BuiltOpenH264)))];
        Self::start_with(drivers, ios, &mobile, Some(home)).await
    }

    async fn start_over(
        ios: Arc<dyn Driver>,
        driver: Arc<FakeDriver>,
        mobile: &std::path::Path,
        home: Option<tempfile::TempDir>,
    ) -> Self {
        let drivers: Vec<Arc<dyn Driver>> = vec![
            ios,
            Arc::new(Absent {
                platform: MobilePlatform::Android,
                reason: "no Android SDK; install Android Studio, or set ANDROID_HOME to the SDK's directory".into(),
            }),
        ];
        Self::start_with(drivers, driver, mobile, home).await
    }

    async fn start_with(
        drivers: Vec<Arc<dyn Driver>>,
        driver: Arc<FakeDriver>,
        mobile: &std::path::Path,
        home: Option<tempfile::TempDir>,
    ) -> Self {
        let (client_io, server_io) = tokio::io::duplex(256 * 1024);
        let service = Arc::new(DemiMobile::with_drivers(drivers, mobile.parent()));
        let server = AbortOnDropHandle::new(tokio::spawn(serve(server_io, service.clone())));
        let (client, connection) = Client::connect(client_io).await.unwrap();
        let connection = AbortOnDropHandle::new(tokio::spawn(async move {
            // The connection ends with the test; how it ends is not the test's concern.
            let _ended = connection.await;
        }));
        let numbers = AbortOnDropHandle::new(answer_numbers(&client).await.unwrap());
        Self {
            driver,
            service,
            client,
            _numbers: numbers,
            _connection: connection,
            _server: server,
            root: tempfile::tempdir().unwrap(),
            _home: home,
        }
    }

    /// The runner stops the program, as when it updates.
    pub async fn stop(self) {
        self.client.shutdown().await.unwrap();
    }

    /// The program is killed: it stops at once and lets nothing go.
    pub fn kill(self) {
        self.service.devices().killed();
    }

    /// `demi mobile <operation> --json` in `conversation`, as its agent runs it.
    pub async fn run(&self, conversation: &str, operation: &str, args: Value) -> Answer {
        self.run_by(conversation, operation, args, CommandCaller::agent(1)).await
    }

    /// The operation as `caller` runs it: the user's are the panel's controls.
    pub async fn run_by(&self, conversation: &str, operation: &str, args: Value, caller: CommandCaller) -> Answer {
        let (_input, output) = self.begin(conversation, operation, args, caller).await;
        collect(output).await
    }

    /// `demi mobile <operation>` without `--json`, as its agent runs it.
    pub async fn run_text(&self, conversation: &str, operation: &str, args: Value) -> Answer {
        let (_input, output) =
            self.invoke(conversation, operation, args, CommandCaller::agent(1), None).await;
        collect(output).await
    }

    /// The agent's operation, started: the caller may cancel it, as the
    /// runner does when the agent's command is interrupted.
    pub async fn begin(
        &self,
        conversation: &str,
        operation: &str,
        args: Value,
        caller: CommandCaller,
    ) -> (demi_command_sdk::CommandInput, demi_command_sdk::CommandOutput) {
        self.invoke(conversation, operation, args, caller, Some(true)).await
    }

    async fn invoke(
        &self,
        conversation: &str,
        operation: &str,
        args: Value,
        caller: CommandCaller,
        json: Option<bool>,
    ) -> (demi_command_sdk::CommandInput, demi_command_sdk::CommandOutput) {
        let request = Invocation {
            operation: format!("mobile.{operation}"),
            invocation_id: format!("{conversation}-{operation}-{}", rand_id()),
            command: format!("demi mobile {operation}"),
            context: CommandContext {
                conversation: conversation.into(),
                caller,
                locale: CommandLocale {
                    time_zone: "UTC".into(),
                    languages: vec!["en-US".into()],
                },
                color_scheme: demi_command_protocol::ColorScheme::Light,
            },
            args,
            cwd: self.root.path().to_str().unwrap().into(),
            env: BTreeMap::new(),
            edits: None,
            json,
            live_input: None,
            viewable: None,
        };
        self.client.invoke(&request).await.unwrap()
    }

    /// The runner releases `conversation`, as it does when the conversation closes.
    pub async fn release(&self, conversation: &str) {
        let (_input, output) = self
            .client
            .conversation(&ConversationRequest::Release {
                conversation: conversation.into(),
            })
            .await
            .unwrap();
        let answer = collect(output).await;
        assert_eq!(answer.exit_code, 0, "{}", answer.stderr);
    }

    /// Opens `device` and answers its handle.
    pub async fn open(&self, conversation: &str, device: &str) -> String {
        let opened = self.run(conversation, "open", serde_json::json!({ "device": device })).await.json();
        opened["device"].as_str().unwrap().to_owned()
    }

    /// The installation the service runs in.
    pub fn home(&self) -> std::path::PathBuf {
        self._home.as_ref().expect("the service has an installation").path().to_owned()
    }

    /// What reached the driver and its devices, in order.
    pub fn log(&self) -> Vec<String> {
        self.driver.log.lock().unwrap().clone()
    }
}

pub async fn collect(mut output: demi_command_sdk::CommandOutput) -> Answer {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut completion: Option<Completion> = None;
    while let Some(record) = output.next().await.unwrap() {
        match record {
            Record::Stdout(bytes) => stdout.extend_from_slice(&bytes),
            Record::Stderr(bytes) => stderr.extend_from_slice(&bytes),
            Record::Completion(value) => completion = Some(value),
            Record::InputPull => panic!("demi mobile reads no stdin"),
            Record::Medium { .. } | Record::MediumBytes(_) => panic!("no media in these calls"),
        }
    }
    let completion = completion.expect("the call completes");
    Answer {
        exit_code: completion.exit_code,
        stdout,
        stderr: String::from_utf8(stderr).unwrap(),
    }
}

fn rand_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// An element as the accessibility service reads it, in points.
pub fn element(role: &str, label: &str, frame: (f64, f64, f64, f64)) -> RawElement {
    RawElement {
        role: role.into(),
        label: Some(label.into()),
        frame: Rect {
            x: frame.0,
            y: frame.1,
            width: frame.2,
            height: frame.3,
        },
        enabled: Some(true),
        ..RawElement::default()
    }
}


impl Mobile {
    /// Opens the `mobile` stream as the page does.
    pub async fn live(&self, conversation: &str) -> (demi_command_sdk::CommandInput, demi_command_sdk::CommandOutput) {
        let request = Invocation {
            operation: "mobile.live".into(),
            invocation_id: format!("{conversation}-live-{}", rand_id()),
            command: "mobile.live".into(),
            context: CommandContext {
                conversation: conversation.into(),
                caller: CommandCaller::User {},
                locale: CommandLocale {
                    time_zone: "UTC".into(),
                    languages: vec!["en-US".into()],
                },
                color_scheme: demi_command_protocol::ColorScheme::Light,
            },
            args: serde_json::json!({}),
            cwd: self.root.path().to_str().unwrap().into(),
            env: BTreeMap::new(),
            edits: None,
            json: None,
            live_input: None,
            viewable: None,
        };
        self.client.invoke(&request).await.unwrap()
    }
}
