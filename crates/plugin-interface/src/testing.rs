//! What every plugin's tests run through (`plugins.md` § One contract, two
//! transports): a loopback that encodes each request, reply and port message
//! to JSON and decodes it again, so a plugin that relied on something only a
//! call in process can carry fails its tests; and [`TestDemi`], which
//! answers the port's operations from memory.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;

use demi_command_declarations::NativeOperation;
use demi_host_interface::{PortError, PortTransport};
use demi_shared_types::Timestamp;
use demi_web_api_protocol::exposes::ExposeAddress;
use demi_web_api_protocol::ids::{DeviceId, ExposeId};
use futures_util::future::LocalBoxFuture;
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use tokio_util::sync::CancellationToken;

use crate::{
    CallKind, ConversationHost, ExposeList, ExposeRecord, ExposeRefusal, Plugin, PluginError,
    PluginPort, PluginTransport, PortAnswer, PortMessage, PortRefusal, Reply, Request, StoredValue,
};

/// `plugin` behind the JSON loopback.
pub fn loopback(plugin: Rc<dyn Plugin>) -> Rc<dyn Plugin> {
    Rc::new(Loopback(plugin))
}

/// A port whose rpc messages `rpc` answers and whose other operations a
/// fresh [`TestDemi`] does.
pub fn port(rpc: Rc<dyn PortTransport>) -> PluginPort {
    TestDemi::with_rpc(rpc).port()
}

/// The value `value` becomes after a wire: encoded to JSON and decoded.
fn across<T: Serialize + DeserializeOwned>(value: &T) -> T {
    let json = serde_json::to_value(value).expect("a plugin message encodes as JSON");
    serde_json::from_value(json).expect("a plugin message decodes from its JSON")
}

struct Loopback(Rc<dyn Plugin>);

impl Plugin for Loopback {
    fn call(
        &self,
        request: Request,
        port: PluginPort,
    ) -> LocalBoxFuture<'_, Result<Reply, PluginError>> {
        Box::pin(async move {
            let port = PluginPort::new(
                Rc::new(JsonMessages(port.transport().clone())),
                port.cancellation().clone(),
            );
            let answer = self.0.call(across(&request), port).await;
            across(&answer)
        })
    }
}

/// A port transport whose messages and answers cross the loopback.
struct JsonMessages(Rc<dyn PluginTransport>);

impl PluginTransport for JsonMessages {
    fn request(&self, message: PortMessage) -> LocalBoxFuture<'_, Result<PortAnswer, PortError>> {
        Box::pin(async move {
            let answer = self.0.request(across(&message)).await?;
            Ok(across(&answer))
        })
    }
}

/// How a test answers a package call.
pub type PackageCalls =
    Box<dyn Fn(&NativeOperation, &Map<String, Value>, CallKind) -> Result<Value, PortRefusal>>;

/// A package call a plugin made.
#[derive(Debug, Clone, PartialEq)]
pub struct PackageCall {
    pub operation: NativeOperation,
    pub args: Map<String, Value>,
    pub kind: CallKind,
}

/// Demi's side of a plugin's port, in memory: an rpc transport, the
/// plugin's values, the conversation's Hosts, the user's exposes on a clock
/// the test sets, the package calls a test answers, and how often the
/// plugin marked its state as changed.
pub struct TestDemi {
    /// What answers a command's rpc messages, which a test may replace
    /// before each command.
    pub rpc: RefCell<Option<Rc<dyn PortTransport>>>,
    values: RefCell<BTreeMap<String, StoredValue>>,
    pub hosts: RefCell<Vec<ConversationHost>>,
    /// Whether the instance has an expose domain.
    pub exposes_available: Cell<bool>,
    exposes: RefCell<Vec<ExposeRecord>>,
    exposes_made: Cell<u64>,
    pub now: Cell<Timestamp>,
    pub package_calls: RefCell<Option<PackageCalls>>,
    /// Each package call the plugin made.
    pub called: RefCell<Vec<PackageCall>>,
    changed: Cell<u32>,
}

impl TestDemi {
    pub fn new() -> Rc<Self> {
        Rc::new(Self::empty(None))
    }

    pub fn with_rpc(rpc: Rc<dyn PortTransport>) -> Rc<Self> {
        Rc::new(Self::empty(Some(rpc)))
    }

    fn empty(rpc: Option<Rc<dyn PortTransport>>) -> Self {
        Self {
            rpc: RefCell::new(rpc),
            values: RefCell::default(),
            hosts: RefCell::default(),
            exposes_available: Cell::new(true),
            exposes: RefCell::default(),
            exposes_made: Cell::new(0),
            now: Cell::new(Timestamp::UNIX_EPOCH),
            package_calls: RefCell::default(),
            called: RefCell::default(),
            changed: Cell::new(0),
        }
    }

    /// A port of one request over this Demi.
    pub fn port(self: &Rc<Self>) -> PluginPort {
        PluginPort::new(self.clone(), CancellationToken::new())
    }

    /// How many times the plugin marked its state as changed.
    pub fn changes(&self) -> u32 {
        self.changed.get()
    }

    /// The live exposes, as the port lists them.
    pub fn live_exposes(&self) -> Vec<ExposeRecord> {
        let now = self.now.get();
        let mut exposes = self.exposes.borrow_mut();
        exposes.retain(|expose| expose.expires_at > now);
        exposes.sort_by_key(|expose| expose.expires_at);
        exposes.clone()
    }

    /// Ends every expose on `device`, as a Cloud's stop does.
    pub fn end_exposes_on(&self, device: &DeviceId) {
        self.exposes
            .borrow_mut()
            .retain(|expose| expose.device != *device);
    }

    fn answer(&self, message: PortMessage) -> Result<PortAnswer, PortRefusal> {
        let answer = match message {
            PortMessage::Rpc { .. } => unreachable!("rpc messages go to the rpc transport"),
            PortMessage::ReadValue { key } => PortAnswer::Value {
                value: self.values.borrow().get(&key).cloned(),
            },
            PortMessage::ListValues => PortAnswer::Values {
                values: self.values.borrow().clone(),
            },
            PortMessage::WriteValue {
                key,
                value,
                revision,
            } => {
                let mut values = self.values.borrow_mut();
                let stored = values.get(&key).map(|stored| stored.revision);
                if stored != revision {
                    return Err(PortRefusal::Conflict);
                }
                let revision = revision.map_or(1, |revision| revision + 1);
                values.insert(key, StoredValue { value, revision });
                PortAnswer::Written { revision }
            }
            PortMessage::Changed => {
                self.changed.set(self.changed.get() + 1);
                PortAnswer::Done
            }
            PortMessage::PackageCall {
                operation,
                args,
                kind,
            } => {
                let calls = self.package_calls.borrow();
                let answer = calls.as_ref().expect("the test answers package calls");
                let result = answer(&operation, &args, kind);
                self.called.borrow_mut().push(PackageCall {
                    operation,
                    args,
                    kind,
                });
                PortAnswer::Called { result: result? }
            }
            PortMessage::ConversationHosts => PortAnswer::Hosts {
                hosts: self.hosts.borrow().clone(),
            },
            PortMessage::ListExposes => PortAnswer::Exposes {
                list: ExposeList {
                    available: self.exposes_available.get(),
                    listed_at: self.now.get(),
                    exposes: if self.exposes_available.get() {
                        self.live_exposes()
                    } else {
                        Vec::new()
                    },
                },
            },
            PortMessage::CreateExpose {
                device,
                address,
                lifetime,
            } => PortAnswer::Expose {
                expose: self.create_expose(device, address, lifetime)?,
            },
            PortMessage::RenewExpose { expose, lifetime } => {
                self.live(&expose)?;
                let mut exposes = self.exposes.borrow_mut();
                let record = exposes
                    .iter_mut()
                    .find(|record| record.id == expose)
                    .expect("a live expose is listed");
                record.expires_at = self.after(lifetime);
                PortAnswer::Expose {
                    expose: record.clone(),
                }
            }
            PortMessage::RemoveExpose { expose } => {
                self.live(&expose)?;
                self.exposes
                    .borrow_mut()
                    .retain(|record| record.id != expose);
                PortAnswer::Done
            }
        };
        Ok(answer)
    }

    fn create_expose(
        &self,
        device: DeviceId,
        address: String,
        lifetime: u64,
    ) -> Result<ExposeRecord, PortRefusal> {
        if !self.exposes_available.get() {
            return Err(expose_refusal(
                ExposeRefusal::Unavailable,
                "no expose domain",
            ));
        }
        let address = ExposeAddress::try_from(address)
            .map_err(|error| expose_refusal(ExposeRefusal::InvalidAddress, error.to_string()))?;
        let hosts = self.hosts.borrow();
        let host = hosts
            .iter()
            .find(|host| host.device == device)
            .ok_or_else(|| expose_refusal(ExposeRefusal::DeviceNotFound, "No such device"))?;
        if !host.online {
            return Err(expose_refusal(ExposeRefusal::DeviceOffline, "offline"));
        }
        let made = self.exposes_made.get() + 1;
        self.exposes_made.set(made);
        let id = test_expose_id(made);
        let record = ExposeRecord {
            url: format!("http://{id}.expose.localhost:3271/"),
            id,
            device,
            device_name: host.name.clone(),
            address,
            created_at: self.now.get(),
            expires_at: self.after(lifetime),
        };
        self.exposes.borrow_mut().push(record.clone());
        Ok(record)
    }

    fn live(&self, expose: &ExposeId) -> Result<(), PortRefusal> {
        if self
            .live_exposes()
            .iter()
            .any(|record| record.id == *expose)
        {
            Ok(())
        } else {
            Err(expose_refusal(
                ExposeRefusal::NotFound,
                format!("No expose {expose}"),
            ))
        }
    }

    fn after(&self, seconds: u64) -> Timestamp {
        let millis = i64::try_from(seconds).expect("a test lifetime fits") * 1000;
        Timestamp::from_millisecond(self.now.get().as_millisecond() + millis)
            .expect("a test expiry is a timestamp")
    }
}

impl PluginTransport for TestDemi {
    fn request(&self, message: PortMessage) -> LocalBoxFuture<'_, Result<PortAnswer, PortError>> {
        Box::pin(async move {
            if let PortMessage::Rpc { request } = message {
                let rpc = self.rpc.borrow().clone();
                let rpc = rpc.expect("the test gives an rpc transport");
                let response = rpc.request(request).await?;
                return Ok(PortAnswer::Rpc { response });
            }
            Ok(self
                .answer(message)
                .unwrap_or_else(|refusal| PortAnswer::Refused { refusal }))
        })
    }
}

fn expose_refusal(reason: ExposeRefusal, message: impl Into<String>) -> PortRefusal {
    PortRefusal::Expose {
        reason,
        message: message.into(),
    }
}

/// The `n`th expose id a [`TestDemi`] draws: 26 base32 characters.
fn test_expose_id(n: u64) -> ExposeId {
    const ALPHABET: &[u8; 32] = b"abcdefghijklmnopqrstuvwxyz234567";
    let mut text = vec![b'a'; 26];
    let mut rest = n;
    for byte in text.iter_mut().rev() {
        *byte = ALPHABET[usize::try_from(rest % 32).expect("a digit fits")];
        rest /= 32;
    }
    ExposeId::try_from(String::from_utf8(text).expect("base32 is ASCII"))
        .expect("26 base32 characters are an expose id")
}

/// A plugin's command lines as a runner reads them, for the plugins' tests:
/// the plugin's trees placed as the plugin host places them, pinned to a
/// package descriptor nobody checks.
pub mod command_line {
    use std::convert::Infallible;

    use demi_command_declarations::{Group, Node, Parsed, UsageError};

    use crate::{DEMI_ROOT, DEMI_SUMMARY, Manifest, Placement};

    /// The roots `manifest`'s commands make: a `demi` root for the groups it
    /// places there, and each root of its own.
    pub fn roots(manifest: &Manifest) -> Vec<Node> {
        let mut demi = Vec::new();
        let mut roots = Vec::new();
        for commands in &manifest.commands {
            match commands.placement {
                Placement::Demi => demi.push(commands.tree.clone()),
                Placement::Root => roots.push(commands.tree.clone()),
            }
        }
        if !demi.is_empty() {
            roots.insert(
                0,
                Node::Group(Group {
                    name: DEMI_ROOT.into(),
                    summary: DEMI_SUMMARY.into(),
                    subcommands: demi,
                }),
            );
        }
        roots
            .into_iter()
            .map(|root| {
                root.pin(&mut |_| Ok::<_, Infallible>("0".repeat(64)))
                    .expect("a declaration pins")
            })
            .collect()
    }

    /// The input `<root> <line>` gives its command, with `stdin` as the body.
    pub fn parse(root: &Node, line: &[&str], stdin: Option<&str>) -> Result<Parsed, UsageError> {
        let argv = argv(line);
        let selected = root.select(&argv)?;
        let parsed = selected.parse(&argv)?;
        match selected.node.leaf() {
            Some(leaf) if !parsed.help => parsed.validate(leaf, stdin.map(str::to_owned)),
            _ => Ok(parsed),
        }
    }

    /// What `<root> <line> --help` prints.
    pub fn help(root: &Node, line: &[&str]) -> String {
        let selected = root.select(&argv(line)).expect("the line names a command");
        selected.node.help(&selected.path.join(" "))
    }

    pub fn argv(line: &[&str]) -> Vec<String> {
        line.iter().map(|word| (*word).to_owned()).collect()
    }
}
