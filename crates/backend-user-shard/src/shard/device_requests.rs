//! The requests the backend makes for the user from one of the user's
//! devices: a provider entry's that connects through it
//! (`crates/provider-common/src/client.rs`).
//!
//! Each request goes to the device's runner, which makes it to its URL as any
//! program on the device does (`crates/runner/src/device_requests.rs`), and its
//! body travels as pipes each way. The runner reaches what the device reaches:
//! the device is its owner's, who runs any command on it already.
//!
//! # Who a device serves
//!
//! - An entry's requests go out from its owner's device, through the owner's
//!   shard.
//! - On a shared instance, an administrator's shared entry that connects
//!   through the administrator's device serves every user of the entry.
//!
//! # Lifetime and failure
//!
//! - A device its owner removed refuses the request: the entry fails until
//!   another is chosen.
//! - An offline device, or a stopped Cloud, which no request wakes, refuses it
//!   at once with that headline, not the unreachable endpoint's, since the
//!   user's next step is to bring the device online. It is retried as an
//!   unreachable endpoint is.
//! - A request whose caller stops waiting ends on the device too.

use std::sync::Arc;

use bytes::Bytes;
use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_device_interface::DeviceErrorKind;
use demi_provider_common::{DeviceAnswer, DeviceRefusal, DeviceRequest, DeviceRoute};
use demi_provider_registry::entry::ProviderEntry;
use demi_runner_client::HttpRequest;
use demi_runner_protocol::device_requests::HttpHeader;
use demi_shared_types::OfflineDevice;
use demi_web_api_protocol::ids::{DeviceId, UserId};
use futures_util::StreamExt as _;
use futures_util::future::BoxFuture;
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use tokio_util::sync::CancellationToken;

use super::{Shard, Shards};

impl Shard {
    /// A request the backend makes for the user from `request`'s device, one
    /// of the user's devices, through its runner.
    async fn device_request(&self, request: DeviceRequest, cancel: CancellationToken) -> Result<DeviceAnswer, DeviceRefusal> {
        let devices = self
            .device_records()
            .await
            .map_err(|error| DeviceRefusal::Failed(error.to_string()))?;
        let device = devices
            .into_iter()
            .find(|device| device.id.as_str() == request.device)
            .ok_or(DeviceRefusal::Removed)?;
        // A stopped Cloud has no connection either: no request wakes it.
        let Some(link) = self.devices().link(&device.id) else {
            return Err(DeviceRefusal::Offline { device: device.name });
        };
        let mut headers = Vec::new();
        for (name, value) in &request.headers {
            let Ok(value) = value.to_str() else {
                return Err(DeviceRefusal::Failed(format!("The {name} header is not text")));
            };
            headers.push(HttpHeader {
                name: name.as_str().to_owned(),
                value: value.to_owned(),
            });
        }
        let body = match request.body.filter(|body| !body.is_empty()) {
            Some(body) => Some(self.send_body(&device.id, body).map_err(|error| DeviceRefusal::Failed(error.to_string()))?),
            None => None,
        };
        let answer = self.pipes().from_device(device.id.as_str());
        let reader = match answer.reader() {
            // A caller that stops reading, a turn stopped or a test ended,
            // fails the pipe, which tells the runner to abandon the request.
            Ok(reader) => reader.failing_when_dropped("the request's caller stopped waiting"),
            Err(error) => return Err(DeviceRefusal::Failed(error.to_string())),
        };
        let sent = HttpRequest {
            url: request.url.to_string(),
            method: request.method.to_string(),
            headers,
        };
        let head = tokio::select! {
            () = cancel.cancelled() => {
                reader.fail("the request was cancelled");
                return Err(DeviceRefusal::Failed("The request was cancelled".into()));
            }
            // The runner answers every request, `http_failed` with its own
            // words when the address or the answer pipe failed, and a lost
            // connection answers it offline.
            head = link.http_request(sent, body, answer.wire_ref()) => head,
        };
        let head = match head {
            Ok(head) => head,
            Err(error) => {
                reader.fail(&error.message);
                return Err(match error.kind {
                    DeviceErrorKind::Offline => DeviceRefusal::Offline { device: device.name },
                    _ => DeviceRefusal::Failed(error.message),
                });
            }
        };
        let Ok(status) = StatusCode::from_u16(head.status) else {
            reader.fail("the answer's status is no HTTP status");
            return Err(DeviceRefusal::Failed(format!("{} is no HTTP status", head.status)));
        };
        let mut answer_headers = HeaderMap::new();
        for header in head.headers {
            // The runner sends only headers it read as such.
            if let (Ok(name), Ok(value)) = (
                HeaderName::from_bytes(header.name.as_bytes()),
                HeaderValue::from_str(&header.value),
            ) {
                answer_headers.append(name, value);
            }
        }
        let body = futures_util::stream::unfold(reader, |mut reader| async move {
            let chunk = reader.next().await?.map_err(std::io::Error::from);
            Some((chunk, reader))
        });
        Ok(DeviceAnswer {
            status,
            headers: answer_headers,
            body: body.boxed(),
        })
    }

    /// A pipe that carries `body` to `device`'s runner, written by a task
    /// of the shard's that ends with the pipe.
    fn send_body(
        &self,
        device: &DeviceId,
        body: Bytes,
    ) -> Result<demi_runner_protocol::wire::PipeRef, demi_runner_client::PipeError> {
        let pipe = self.pipes().to_device(device.as_str());
        let mut writer = pipe.writer()?;
        self.tasks.spawn_local(async move {
            // A pipe that failed has told its runner already.
            if writer.write(body).await.is_ok() {
                writer.end();
            }
        });
        Ok(pipe.wire_ref())
    }
}

/// The device `entry` connects through, which a failure of its request
/// names as offline. A read that fails adds nothing: the fact only adds to
/// the failure.
pub(crate) async fn through_device(control: &ControlService, entry: &ProviderEntry) -> Option<OfflineDevice> {
    let id = entry.through_device.as_ref()?;
    let read = async {
        let mut devices = control.paired_devices(entry.owner.clone()).await?;
        devices.extend(control.managed_device(entry.owner.clone()).await?);
        Ok::<_, StorageError>(devices)
    };
    let devices = match read.await {
        Ok(devices) => devices,
        Err(error) => {
            tracing::warn!(event = "provider.through_device_read_failed", provider = %entry.id, error = &error as &dyn std::error::Error, "the device a provider connects through was not read");
            return None;
        }
    };
    let device = devices.into_iter().find(|device| device.id == *id)?;
    Some(OfflineDevice {
        device_id: device.id.into_string(),
        device: device.name,
    })
}

/// The backend's way to its users' devices for the provider client: each
/// request goes to the shard of the device's owner.
pub struct DeviceRequests {
    shards: Shards,
}

impl DeviceRequests {
    pub fn new(shards: Shards) -> Arc<Self> {
        Arc::new(Self { shards })
    }
}

impl DeviceRoute for DeviceRequests {
    fn send(&self, owner: &str, request: DeviceRequest) -> BoxFuture<'static, Result<DeviceAnswer, DeviceRefusal>> {
        let shards = self.shards.clone();
        let owner = UserId::try_from(owner.to_owned());
        Box::pin(async move {
            // An owner that is no user's id has no devices.
            let Ok(owner) = owner else {
                return Err(DeviceRefusal::Removed);
            };
            shards
                .of(&owner)
                .call(move |shard, cancel| async move { shard.device_request(request, cancel).await })
                .await
                .unwrap_or_else(|error| Err(DeviceRefusal::Failed(error.to_string())))
        })
    }
}
