//! The `demi.mobile` package: the Host's simulators for the conversations that
//! open them, as a resident command service, with a driver per platform behind
//! one interface (`crates/plugin-mobile/command/src/driver.rs`).
//!
//! The agent runs the user's iOS or Android app in a simulator on a paired
//! device and operates it as a person would, while the user watches and
//! operates the same simulator live in the work panel:
//!
//! ```text
//! web page                    backend             Host (a paired device)
//!   device tabs, toolbar  <-- plugin-mobile --> demi-mobile (this crate)
//!   live view  <------- `mobile` user stream --->   Devices: who has what
//!                                                   Driver, one per platform
//!                                                     ios: demi-ios-sim helper
//!                                                     android: emulator gRPC,
//!                                                              u2.jar over adb
//! ```
//!
//! # What Demi does
//!
//! Demi does what touches the screen, the input and who controls them, each the
//! one operation behind both the panel's control and its command, so the agent
//! can do what the user can and the other way round.
//!
//! Everything else is left to the tools the agent already knows (`xcodebuild`,
//! `xcrun simctl`, `avdmanager`, `emulator`, `adb`): building, installing,
//! launching an app, logs, locale, permissions, and creating or deleting
//! simulators.
//!
//! # What is supported
//!
//! - Only simulators are supported: an iPhone takes no input without a signed
//!   XCTest runner on it, and a phone holds a person's real accounts.
//! - Only paired devices run them, since a Cloud has neither Xcode nor the
//!   hardware virtualization the Android Emulator needs.
//! - Xcode and the Android SDK are the user's. Demi's own helpers, the iOS one
//!   and the Android one, ship with the runner.
//!
//! The package is a program of its own: it hosts no runner connection, defines
//! no part of the agent's command tree, stores no conversation, and is never
//! linked into the runner.

pub mod android;
pub mod devices;
pub mod driver;
mod error;
mod frame;
pub mod ios;
pub mod kept;
mod keys;
mod live;
mod operations;
mod pictures;
mod refs;
mod scale;
#[cfg(feature = "testing")]
pub mod testing;
mod text;
mod tree;

use std::{future::Future, pin::Pin, sync::Arc};

use bytes::Bytes;

use demi_command_package_mobile_protocol::{
    Operation, OperationError,
    mobile::{FailureDocument, MobileFailure},
};
use demi_command_protocol::{CommandError, Completion, ConversationRequest, ConversationStatus, Invocation};
use demi_command_sdk::{ConversationContext, Handler, InvocationContext, Messages, Numbers, ServiceError};

pub use error::MobileError;
pub use operations::{QUIET, SETTLE_LIMIT};

use crate::{devices::Devices, driver::Driver, live::Hub, operations::Produced};

/// The service.
pub struct DemiMobile {
    devices: Arc<Devices>,
    hub: Arc<Hub>,
    /// The installation, `DEMI_HOME`, whose `scratch/<12 digits of the conversation's id>` is each
    /// conversation's scratch directory; none for a service without one.
    home: Option<std::path::PathBuf>,
}

impl DemiMobile {
    /// The service with this Host's drivers.
    pub fn new() -> Self {
        // The installation's `mobile/`, where the simulators Demi booted
        // and the open devices are kept across the program's ends (`crates/plugin-mobile/command/src/devices.rs`).
        let home = std::env::var_os("DEMI_HOME").map(std::path::PathBuf::from);
        Self::with_drivers(host_drivers(home.as_deref()), home.as_deref())
    }

    /// The service with `drivers`, such as a test's, in the installation
    /// `home`: it keeps the simulators it boots in `home/mobile`, and each
    /// conversation's files in `home/scratch/<12 digits of the conversation's id>`.
    pub fn with_drivers(drivers: Vec<Arc<dyn Driver>>, home: Option<&std::path::Path>) -> Self {
        let mobile = home.map(|home| home.join("mobile"));
        // A conversation is still on this Host while its scratch directory
        // is: the runner removes the directories of the conversations the
        // backend no longer lists as it connects
        // (`crates/runner-jobs/src/scratch.rs`).
        let scratch = home.map(|home| home.join("scratch"));
        let exists = Box::new(move |conversation: &str| {
            scratch
                .as_ref()
                .is_none_or(|scratch| scratch.join(demi_command_protocol::scratch_name(conversation)).is_dir())
        });
        Self {
            devices: Devices::new(drivers, kept::Kept::load(mobile.as_deref()), exists),
            hub: Arc::default(),
            home: home.map(std::path::Path::to_owned),
        }
    }

    /// `conversation`'s scratch directory on this Host, or the system's
    /// temporary directory's for a service without an installation.
    fn scratch(&self, conversation: &str) -> std::path::PathBuf {
        self.home
            .clone()
            .unwrap_or_else(std::env::temp_dir)
            .join("scratch")
            .join(demi_command_protocol::scratch_name(conversation))
    }

    pub fn devices(&self) -> &Arc<Devices> {
        &self.devices
    }
}

impl Default for DemiMobile {
    fn default() -> Self {
        Self::new()
    }
}

/// The drivers this Host has: iOS on a Mac, Android where its SDK is. An
/// emulator Demi boots writes its log into the installation's
/// `mobile/android`, which keeps Cisco's OpenH264 too.
fn host_drivers(home: Option<&std::path::Path>) -> Vec<Arc<dyn Driver>> {
    vec![
        #[cfg(target_os = "macos")]
        Arc::new(ios::Ios::new()),
        #[cfg(not(target_os = "macos"))]
        Arc::new(driver::Absent {
            platform: demi_command_package_mobile_protocol::mobile::MobilePlatform::Ios,
            reason: "iOS simulators need a Mac".into(),
        }),
        Arc::new(android::Android::new(home.map(|home| home.join("mobile").join("android")))),
    ]
}

impl Handler for DemiMobile {
    type Metadata = Invocation;

    fn operations(&self) -> Vec<String> {
        Operation::names().map(String::from).collect()
    }

    fn numbers(&self, numbers: Numbers) {
        self.devices.attach_numbers(numbers);
    }

    fn messages(&self, messages: Messages) {
        self.devices.attach_messages(messages);
    }

    fn conversation(
        &self,
        context: ConversationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        let devices = self.devices.clone();
        Box::pin(async move {
            let body = match &context.request {
                ConversationRequest::Release { conversation } => {
                    devices.release(conversation).await;
                    Bytes::from_static(b"{}")
                }
                ConversationRequest::Status {} => Bytes::from(serde_json::to_vec(&ConversationStatus {
                    conversations: devices.conversations().await,
                })?),
            };
            context.output.stdout(body).await?;
            Ok(Completion {
                exit_code: 0,
                error: None,
            })
        })
    }

    fn close(&self) -> Pin<Box<dyn Future<Output = Result<(), ServiceError>> + Send>> {
        let devices = self.devices.clone();
        let hub = self.hub.clone();
        Box::pin(async move {
            hub.close().await;
            devices.close_all().await;
            Ok(())
        })
    }

    fn invoke(
        &self,
        context: InvocationContext,
    ) -> Pin<Box<dyn Future<Output = Result<Completion, ServiceError>> + Send>> {
        let devices = self.devices.clone();
        let hub = self.hub.clone();
        let installed = self.home.is_some();
        let scratch = self.scratch(&context.request.context.conversation);
        devices.reached(&context.request.context.conversation);
        Box::pin(async move {
            let operation = match Operation::parse(&context.request.operation, context.request.args.clone()) {
                Ok(Operation::Live) => return live::serve(devices, hub, context).await,
                Ok(Operation::Mobile(operation)) => Ok(*operation),
                Err(OperationError::Unknown(operation)) => {
                    return Err(ServiceError::UnknownOperation(operation));
                }
                Err(OperationError::Invalid(error)) => Err(MobileError::Usage(error.to_string())),
            };
            let json = context.request.json == Some(true);
            let call = operations::Call {
                conversation: &context.request.context.conversation,
                command: &context.request.command,
                cwd: &context.request.cwd,
                // The panel saves what the user asks for where the page
                // downloads it from; the agent's files go where it says.
                scratch: (installed && context.request.context.caller.agent_number().is_none())
                    .then(|| scratch.clone()),
                recordings: scratch.clone(),
                agent: context.request.context.caller.agent_number().is_some(),
                cancel: &context.cancellation,
            };
            let outcome = match operation {
                Ok(operation) => operations::run(&devices, &call, operation).await,
                Err(error) => Err(error),
            };
            match outcome {
                Ok(outcome) => {
                    for note in outcome.notes {
                        context.output.stderr(Bytes::from(format!("{note}\n"))).await?;
                    }
                    match outcome.produced {
                        Produced::Result { json: value, text } => {
                            let bytes = if json {
                                serde_json::to_vec(&value)?
                            } else {
                                text.into_bytes()
                            };
                            context.output.stdout(Bytes::from(bytes)).await?;
                        }
                        Produced::Bytes { bytes, about } => {
                            context.output.stderr(Bytes::from(format!("{about}\n"))).await?;
                            context.output.stdout(bytes).await?;
                        }
                        Produced::File { file, about } => {
                            let written = stream_file(&context, &file, &about).await;
                            operations::discard(&file).await;
                            written?;
                        }
                    }
                    Ok(Completion {
                        exit_code: 0,
                        error: None,
                    })
                }
                Err(MobileError::Cancelled) => Err(ServiceError::Cancelled),
                Err(error) => {
                    let code = error.code();
                    let exit_code = error.exit_code();
                    let message = error.to_string();
                    if json {
                        let document = serde_json::to_vec(&FailureDocument {
                            error: MobileFailure { code, message },
                        })?;
                        context.output.stderr(Bytes::from(document)).await?;
                        return Ok(Completion {
                            exit_code,
                            error: None,
                        });
                    }
                    Ok(Completion {
                        exit_code,
                        error: Some(CommandError {
                            code: code.to_string(),
                            message,
                        }),
                    })
                }
            }
        })
    }
}

/// Writes `file` to standard output a piece at a time, `about` first on
/// standard error.
async fn stream_file(context: &InvocationContext, file: &std::path::Path, about: &str) -> Result<(), ServiceError> {
    use tokio::io::AsyncReadExt;
    context.output.stderr(Bytes::from(format!("{about}\n"))).await?;
    let mut opened = tokio::fs::File::open(file).await?;
    let mut buffer = vec![0; 256 * 1024];
    loop {
        let count = opened.read(&mut buffer).await?;
        if count == 0 {
            return Ok(());
        }
        context.output.stdout(Bytes::copy_from_slice(&buffer[..count])).await?;
    }
}

