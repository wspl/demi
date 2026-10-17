//! The `demi.mobile` package (`crates-and-packages.md`
//! § command-package-mobile): the Host's simulators for the conversations
//! that open them, as a resident command service, with a driver per
//! platform behind one interface (`mobile.md` § Drivers).

pub mod android;
mod booted;
pub mod devices;
pub mod driver;
mod error;
mod frame;
pub mod ios;
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
    /// The installation, `DEMI_HOME`, whose `scratch/<conversation>` is each
    /// conversation's scratch directory; none for a service without one.
    home: Option<std::path::PathBuf>,
}

impl DemiMobile {
    /// The service with this Host's drivers.
    pub fn new() -> Self {
        // The installation's `mobile/`, where the simulators Demi booted
        // are kept across the program's ends (`mobile.md` § Lifetime).
        let home = std::env::var_os("DEMI_HOME").map(std::path::PathBuf::from);
        Self::with_drivers(host_drivers(home.as_deref()), home.as_deref())
    }

    /// The service with `drivers`, such as a test's, in the installation
    /// `home`: it keeps the simulators it boots in `home/mobile`, and each
    /// conversation's files in `home/scratch/<conversation>`.
    pub fn with_drivers(drivers: Vec<Arc<dyn Driver>>, home: Option<&std::path::Path>) -> Self {
        let mobile = home.map(|home| home.join("mobile"));
        Self {
            devices: Devices::new(drivers, booted::Booted::load(mobile.as_deref())),
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
            .join(conversation)
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
                    conversations: devices.conversations(),
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

