//! The user's exposed ports in their shard: what a
//! device's page in Settings › Devices and `demi host expose` read and
//! change, and the backend's requests for the user to a device's port. Such
//! a request names one of the user's devices by its id, goes only to a port
//! the device exposed, and travels to its runner as pipes; an offline
//! device, or a stopped Cloud, which it never wakes, refuses it at once,
//! from the device's state the shard holds.
//!
//! Exposure is for the backend's requests, such as a provider entry's, not for
//! programs on a Host (those use a port forward); the backend listens on no
//! port for either, since on a shared instance every user's programs on the
//! server could connect to it. The host of the address is the device's id, as
//! a container's name is its host on a Docker network: it is stable while the
//! user renames the device, and no name on the internet has that form, so it
//! never shadows a real host. Only exposed ports are reachable, so a provider
//! entry, a pasted URL or an agent's command can never reach anything else on
//! the device or its network. A device's exposed ports serve the requests made
//! for its owner, and on a shared instance an administrator's shared entry
//! that names the administrator's device serves every user of the entry; an
//! address naming another user's device is an ordinary, unknown host. A
//! redirect is followed as the network's: an address naming a device is an
//! unknown host there, so no redirect reaches a device. Removing a port ends
//! the requests under way through it. A request for a device that is offline
//! fails with that headline, not the unreachable endpoint's, since the user's
//! next step is to bring the device online; it is retried as an unreachable
//! endpoint is.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use demi_backend_database::StorageError;
use demi_backend_database::control::ControlService;
use demi_backend_providers::vault::entries::{EntryCredential, ProviderEntry};
use demi_backend_database::exposed_ports::{ExposedPortChange, ExposedPortRecord, ExposedPortRefusal};
use demi_backend_page_sync::Part;
use demi_backend_remote_host::HttpRequest;
use demi_host_interface::HostErrorKind;
use demi_provider_common::{DeviceAddress, DeviceAnswer, DeviceRefusal, DeviceRequest, DeviceRoute, Routed};
use demi_shared_types::NotExposedPort;
use demi_runner_protocol::exposure::{HttpHeader, HttpScheme};
use demi_web_api_protocol::exposed_ports::{ExposedPortDto, ExposedPortPatch, NewExposedPort, exposed_address};
use demi_web_api_protocol::ids::{DeviceId, ExposedPortId, UserId};
use futures_util::StreamExt as _;
use futures_util::future::BoxFuture;
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};
use tokio_util::sync::CancellationToken;

use super::{Shard, Shards};

/// Why a change of an exposed port was refused.
#[derive(Debug, thiserror::Error)]
pub enum ExposedPortError {
    #[error(transparent)]
    Refused(#[from] ExposedPortRefusal),
    #[error(transparent)]
    Storage(#[from] StorageError),
}

/// The requests under way through each exposed port: a token per port,
/// whose children the requests through it watch, cancelled when the port
/// stops being exposed.
#[derive(Default)]
pub(crate) struct Underway(RefCell<HashMap<(DeviceId, u16), CancellationToken>>);

impl Underway {
    /// What a new request through `port` of `device` watches.
    fn through(&self, device: &DeviceId, port: u16) -> CancellationToken {
        self.0
            .borrow_mut()
            .entry((device.clone(), port))
            .or_default()
            .child_token()
    }

    /// Ends the requests through every port that `exposed` no longer holds.
    fn keep_only(&self, exposed: &[ExposedPortRecord]) {
        self.0.borrow_mut().retain(|(device, port), requests| {
            let still = exposed
                .iter()
                .any(|exposed| exposed.device == *device && exposed.port == Some(*port));
            if !still {
                requests.cancel();
            }
            still
        });
    }
}

fn dto(record: ExposedPortRecord) -> ExposedPortDto {
    ExposedPortDto {
        address: record.port.map(|port| exposed_address(&record.device, port)),
        id: record.id,
        device_id: record.device,
        port: record.port,
        label: record.label,
        created_at: record.created_at,
    }
}

impl Shard {
    /// The user's exposed ports, oldest first.
    pub async fn exposed_port_list(&self) -> Result<Vec<ExposedPortDto>, StorageError> {
        let records = self.services().control.exposed_ports(self.user().clone()).await?;
        Ok(records.into_iter().map(dto).collect())
    }

    /// `POST /exposed-ports` and `demi host expose add`: the port's row,
    /// and whether this call exposed it rather than finding it exposed.
    pub async fn create_exposed_port(&self, new: NewExposedPort) -> Result<(ExposedPortDto, bool), ExposedPortError> {
        let label = new.label.map(|label| label.into_string()).unwrap_or_default();
        let created = self
            .services()
            .control
            .create_exposed_port(self.user().clone(), new.device_id, new.port, label)
            .await??;
        if created.new {
            self.mark(Part::ExposedPorts);
        }
        Ok((dto(created.exposed), created.new))
    }

    /// `PATCH /exposed-ports/:id`.
    pub async fn change_exposed_port(
        &self,
        id: ExposedPortId,
        patch: ExposedPortPatch,
    ) -> Result<ExposedPortDto, ExposedPortError> {
        let change = ExposedPortChange {
            port: patch.port,
            label: patch.label.map(|label| label.into_string()),
        };
        let stored = self
            .services()
            .control
            .change_exposed_port(self.user().clone(), id, change)
            .await??;
        self.mark(Part::ExposedPorts);
        self.end_unexposed().await;
        Ok(dto(stored))
    }

    /// `DELETE /exposed-ports/:id` and `demi host expose remove`.
    pub async fn remove_exposed_port(&self, id: ExposedPortId) -> Result<(), ExposedPortError> {
        let removed = self
            .services()
            .control
            .remove_exposed_port(self.user().clone(), id)
            .await?;
        if !removed {
            return Err(ExposedPortRefusal::NotFound.into());
        }
        self.mark(Part::ExposedPorts);
        self.end_unexposed().await;
        Ok(())
    }

    /// Ends the requests under way through ports no longer exposed, as
    /// turning a forward off ends its connections. A read that fails ends
    /// none: they end with their callers.
    async fn end_unexposed(&self) {
        match self.services().control.exposed_ports(self.user().clone()).await {
            Ok(exposed) => self.exposure.keep_only(&exposed),
            Err(error) => {
                tracing::warn!(error = &error as &dyn std::error::Error, "the requests through removed ports were not ended");
            }
        }
    }

    /// A request the backend makes for the user to `request`'s address: to
    /// the device it names when it is one of the user's Hosts and has
    /// exposed the port, through its runner; an address naming no device
    /// of the user's is an ordinary host.
    async fn device_request(&self, request: DeviceRequest, cancel: CancellationToken) -> Routed {
        let failed = |error: StorageError| Routed::Refused(DeviceRefusal::Failed(error.to_string()));
        let devices = match self.host_records().await {
            Ok(devices) => devices,
            Err(error) => return failed(error),
        };
        let Some(device) = devices
            .into_iter()
            .find(|device| device.id.as_str() == request.address.device)
        else {
            return Routed::NotADevice;
        };
        let port = request.address.port;
        let exposed = match self.services().control.exposed_ports(self.user().clone()).await {
            Ok(exposed) => exposed
                .iter()
                .any(|exposed| exposed.device == device.id && exposed.port == Some(port)),
            Err(error) => return failed(error),
        };
        if !exposed {
            return Routed::Refused(DeviceRefusal::NotExposed { device: device.name, port });
        }
        let offline = || Routed::Refused(DeviceRefusal::Offline { device: device.name.clone() });
        // A stopped Cloud has no connection either: no request to a port
        // wakes it.
        let Some(link) = self.devices().link(&device.id) else {
            return offline();
        };
        let mut headers = Vec::new();
        for (name, value) in &request.headers {
            let Ok(value) = value.to_str() else {
                return Routed::Refused(DeviceRefusal::Failed(format!("The {name} header is not text")));
            };
            headers.push(HttpHeader {
                name: name.as_str().to_owned(),
                value: value.to_owned(),
            });
        }
        let body = match request.body.filter(|body| !body.is_empty()) {
            Some(body) => match self.send_body(&device.id, body) {
                Ok(pipe) => Some(pipe),
                Err(error) => return Routed::Refused(DeviceRefusal::Failed(error.to_string())),
            },
            None => None,
        };
        let answer = self.pipes().from_device(device.id.as_str());
        let reader = match answer.reader() {
            // A caller that stops reading, a turn stopped or a test ended,
            // fails the pipe, which tells the runner to abandon the request.
            Ok(reader) => reader.failing_when_dropped("the request's caller stopped waiting"),
            Err(error) => return Routed::Refused(DeviceRefusal::Failed(error.to_string())),
        };
        let sent = HttpRequest {
            scheme: if request.address.secure { HttpScheme::Https } else { HttpScheme::Http },
            port,
            method: request.method.to_string(),
            path: request.path,
            headers,
        };
        let unexposed = self.exposure.through(&device.id, port);
        let no_longer = || format!("Port {port} of {} is no longer exposed.", device.name);
        let head = tokio::select! {
            () = cancel.cancelled() => {
                reader.fail("the request was cancelled");
                return Routed::Refused(DeviceRefusal::Failed("The request was cancelled".into()));
            }
            () = unexposed.cancelled() => {
                reader.fail("the port is no longer exposed");
                return Routed::Refused(DeviceRefusal::Failed(no_longer()));
            }
            // The runner answers every request, `http_failed` with its own
            // words when the port or the answer pipe failed, and a lost
            // connection answers it offline.
            head = link.http_request(sent, body, answer.wire_ref()) => head,
        };
        let head = match head {
            Ok(head) => head,
            Err(error) => {
                reader.fail(&error.message);
                return match error.kind {
                    HostErrorKind::Offline => offline(),
                    _ => Routed::Refused(DeviceRefusal::Failed(error.message)),
                };
            }
        };
        let Ok(status) = StatusCode::from_u16(head.status) else {
            reader.fail("the answer's status is no HTTP status");
            return Routed::Refused(DeviceRefusal::Failed(format!("{} is no HTTP status", head.status)));
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
        // The body ends early, failed, when the port stops being exposed.
        let ended = no_longer();
        let body = futures_util::stream::unfold(Some((reader, unexposed)), move |state| {
            let ended = ended.clone();
            async move {
                let (mut reader, unexposed) = state?;
                tokio::select! {
                    biased;
                    () = unexposed.cancelled() => {
                        reader.fail("the port is no longer exposed");
                        Some((Err(std::io::Error::other(ended)), None))
                    }
                    chunk = reader.next() => {
                        let chunk = chunk?.map_err(std::io::Error::from);
                        Some((chunk, Some((reader, unexposed))))
                    }
                }
            }
        });
        Routed::Answered(DeviceAnswer {
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
    ) -> Result<demi_runner_protocol::wire::PipeRef, demi_backend_remote_host::PipeError> {
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

/// The device of its owner's that `entry`'s endpoint names, and the port:
/// what a failure of the entry's request to it names, the port to expose
/// or the device that was offline. A
/// read that fails adds nothing: the fact only adds to the failure.
pub(crate) async fn endpoint_device(control: &ControlService, entry: &ProviderEntry) -> Option<NotExposedPort> {
    let EntryCredential::ApiKey(config) = &entry.credential else {
        return None;
    };
    let address = DeviceAddress::of(config.base_url.as_ref()?.url())?;
    let read = async {
        let mut devices = control.paired_devices(entry.owner.clone()).await?;
        devices.extend(control.managed_device(entry.owner.clone()).await?);
        Ok::<_, StorageError>(devices)
    };
    let devices = match read.await {
        Ok(devices) => devices,
        Err(error) => {
            tracing::warn!(provider = %entry.id, error = &error as &dyn std::error::Error, "the device an endpoint names was not read");
            return None;
        }
    };
    let device = devices.into_iter().find(|device| device.id.as_str() == address.device)?;
    Some(NotExposedPort {
        device_id: device.id.into_string(),
        device: device.name,
        port: address.port,
    })
}

/// The backend's way to its users' devices for the provider client: each
/// request goes to the shard of the user whose device the address may name.
pub struct DeviceRequests {
    shards: Shards,
}

impl DeviceRequests {
    pub fn new(shards: Shards) -> Arc<Self> {
        Arc::new(Self { shards })
    }
}

impl DeviceRoute for DeviceRequests {
    fn send(&self, owner: &str, request: DeviceRequest) -> BoxFuture<'static, Routed> {
        let shards = self.shards.clone();
        let owner = UserId::try_from(owner.to_owned());
        Box::pin(async move {
            // An owner that is no user's id has no devices.
            let Ok(owner) = owner else {
                return Routed::NotADevice;
            };
            shards
                .of(&owner)
                .call(move |shard, cancel| async move { shard.device_request(request, cancel).await })
                .await
                .unwrap_or_else(|error| Routed::Refused(DeviceRefusal::Failed(error.to_string())))
        })
    }
}
