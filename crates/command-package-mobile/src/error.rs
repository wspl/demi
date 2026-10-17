//! How an operation fails, in the words the agent reads and with the code a
//! script branches on (`browser.md` § Errors, which the mobile commands
//! follow).

use demi_command_package_mobile_protocol::mobile::{
    DeviceId, ElementRef, MAX_DEVICES, MobileErrorCode,
};

use crate::driver::DriverError;

#[derive(Debug, thiserror::Error)]
pub enum MobileError {
    #[error("{0}")]
    Usage(String),
    #[error("{0} is not open in this conversation on this Host; demi mobile list shows what is")]
    NotOpen(DeviceId),
    #[error("{}", no_simulator(.query, .unavailable))]
    NoSimulator {
        query: String,
        /// The platforms the Host cannot run, with why.
        unavailable: Vec<String>,
    },
    #[error("{}", ambiguous(.name, .candidates))]
    Ambiguous {
        name: String,
        /// Each simulator of the name: its native id, its system and its state.
        candidates: Vec<(String, String, String)>,
    },
    #[error("{0} is open in another conversation; close it there first")]
    Busy(String),
    #[error("this conversation has {MAX_DEVICES} devices open on this Host, the most it may; close one first")]
    TooMany,
    #[error("{0} is not an element of the last inspect; run demi mobile inspect again")]
    StaleRef(ElementRef),
    #[error("{0}")]
    Timeout(String),
    #[error("{0}")]
    Unavailable(String),
    #[error("the device is not recording; start with demi mobile record <m> start")]
    NotRecording,
    #[error("the device is recording already")]
    Recording,
    #[error("{0} exists; pass --overwrite to replace it")]
    OutputExists(String),
    #[error("{0}")]
    Io(String),
    #[error("{0}")]
    Failed(String),
    #[error("{0}")]
    Driver(DriverError),
    #[error("the operation was cancelled")]
    Cancelled,
}

fn no_simulator(query: &str, unavailable: &[String]) -> String {
    let mut message = format!("no simulator named {query:?} on this Host; demi mobile list shows them");
    for reason in unavailable {
        message.push_str("; ");
        message.push_str(reason);
    }
    message
}

fn ambiguous(name: &str, candidates: &[(String, String, String)]) -> String {
    let listed: Vec<String> = candidates
        .iter()
        .map(|(id, os, state)| format!("{id} ({os}, {state})"))
        .collect();
    format!(
        "{} simulators are named {name:?}: {}; open the one you mean by its id",
        listed.len(),
        listed.join(" and ")
    )
}

impl MobileError {
    pub fn code(&self) -> MobileErrorCode {
        match self {
            Self::Usage(_) => MobileErrorCode::InvalidInput,
            Self::NotOpen(_) | Self::NoSimulator { .. } => MobileErrorCode::DeviceNotFound,
            Self::Ambiguous { .. } => MobileErrorCode::AmbiguousDevice,
            Self::Busy(_) => MobileErrorCode::DeviceBusy,
            Self::TooMany => MobileErrorCode::TooManyDevices,
            Self::StaleRef(_) => MobileErrorCode::StaleRef,
            Self::Timeout(_) => MobileErrorCode::Timeout,
            Self::Unavailable(_) => MobileErrorCode::Unavailable,
            Self::NotRecording => MobileErrorCode::NotRecording,
            Self::Recording => MobileErrorCode::Recording,
            Self::OutputExists(_) => MobileErrorCode::OutputExists,
            Self::Io(_) => MobileErrorCode::IoError,
            Self::Failed(_) => MobileErrorCode::DriverError,
            Self::Driver(DriverError::Lost(_)) => MobileErrorCode::DeviceLost,
            Self::Driver(DriverError::Failed(_) | DriverError::NoFocusedField(_)) => MobileErrorCode::DriverError,
            Self::Cancelled => MobileErrorCode::Cancelled,
        }
    }

    /// The exit status it ends the command with: 2 for a usage error, 130
    /// for a cancellation, 1 otherwise.
    pub fn exit_code(&self) -> u8 {
        match self.code() {
            MobileErrorCode::InvalidInput => 2,
            MobileErrorCode::Cancelled => 130,
            _ => 1,
        }
    }
}

impl From<std::io::Error> for MobileError {
    fn from(error: std::io::Error) -> Self {
        Self::Io(error.to_string())
    }
}
