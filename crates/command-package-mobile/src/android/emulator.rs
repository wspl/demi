//! A running emulator's gRPC interface
//! (`crates/command-package-mobile/src/android/mod.rs`): where its
//! discovery file says it listens, authenticated as the emulator asks. An
//! emulator started with JWT authentication, the default, takes a token
//! signed by a key whose public half the client puts into the emulator's
//! key directory; one started with `-grpc-use-token` takes the token its
//! discovery file names.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use aws_lc_rs::{
    rand::SystemRandom,
    signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair},
};
use base64::Engine as _;
use tonic::{
    metadata::MetadataValue,
    service::{Interceptor, interceptor::InterceptedService},
    transport::Channel,
};

use super::proto::emulator_controller_client::EmulatorControllerClient;
use crate::driver::{DriverError, Result};

/// The largest message the emulator sends: a picture at a tablet's full
/// size, 2560 × 1600 RGB, with room.
const LARGEST_MESSAGE: usize = 64 * 1024 * 1024;
/// How long the emulator takes to take a new key in.
const KEY_TAKEN: Duration = Duration::from_secs(10);
/// How long a token lives, and when a new one is signed.
const TOKEN_LIFETIME: Duration = Duration::from_secs(60 * 60);
const TOKEN_RENEWAL: Duration = Duration::from_secs(50 * 60);
/// The issuer the emulator's default allow list grants every method of
/// `EmulatorController` (`<sdk>/emulator/lib/emulator_access.json`).
const ISSUER: &str = "android-studio";

/// A running emulator as its discovery file, `pid_<pid>.ini` in the
/// emulators' running directory, describes it.
#[derive(Debug, Clone, PartialEq)]
pub struct Discovery {
    pub pid: u32,
    /// Its AVD's name.
    pub avd: String,
    pub grpc_port: u16,
    /// The console's port, which names it to adb: `emulator-5554`.
    pub console_port: u16,
    /// Where the client puts its public key, under JWT authentication.
    pub jwks: Option<PathBuf>,
    /// The keys the emulator took in, which it rewrites as it takes one.
    pub jwk_active: Option<PathBuf>,
    /// The token it takes under `-grpc-use-token`.
    pub token: Option<String>,
}

impl Discovery {
    /// The file `text` of the emulator with process `pid`; none for one
    /// that names no AVD or no gRPC port, which Demi cannot drive.
    pub fn parse(pid: u32, text: &str) -> Option<Self> {
        let value = |key: &str| {
            text.lines()
                .filter_map(|line| line.split_once('='))
                .find(|(name, _)| name.trim() == key)
                .map(|(_, value)| value.trim().to_owned())
        };
        Some(Self {
            pid,
            avd: value("avd.name")?,
            grpc_port: value("grpc.port")?.parse().ok()?,
            console_port: value("port.serial")?.parse().ok()?,
            jwks: value("grpc.jwks").map(PathBuf::from),
            jwk_active: value("grpc.jwk_active").map(PathBuf::from),
            token: value("grpc.token"),
        })
    }

    /// Its adb serial.
    pub fn serial(&self) -> String {
        format!("emulator-{}", self.console_port)
    }
}

pub type Client = EmulatorControllerClient<InterceptedService<Channel, Auth>>;

/// A connection to an emulator's interface; the key it registered goes
/// with it.
pub struct Emulator {
    pub client: Client,
    _key: Option<Registered>,
}

impl Emulator {
    pub async fn connect(discovery: &Discovery) -> Result<Self> {
        let (auth, key) = match (&discovery.jwks, &discovery.jwk_active) {
            (Some(jwks), Some(active)) => {
                let signer = Signer::new()?;
                let registered = signer.register(jwks, active).await?;
                (Auth::Signed(Arc::new(signer)), Some(registered))
            }
            _ => match &discovery.token {
                Some(token) => (Auth::Token(bearer(token)?), None),
                None => (Auth::None, None),
            },
        };
        let channel = Channel::from_shared(format!("http://127.0.0.1:{}", discovery.grpc_port))
            .map_err(|error| DriverError::Failed(format!("the emulator's address is not one: {error}")))?
            .connect()
            .await
            .map_err(|error| {
                DriverError::Lost(format!("{} does not answer on its port {}: {error}", discovery.avd, discovery.grpc_port))
            })?;
        let client = EmulatorControllerClient::with_interceptor(channel, auth)
            .max_decoding_message_size(LARGEST_MESSAGE);
        Ok(Self { client, _key: key })
    }
}

fn bearer(token: &str) -> Result<MetadataValue<tonic::metadata::Ascii>> {
    format!("Bearer {token}")
        .parse()
        .map_err(|_| DriverError::Failed("the emulator's token is not one an HTTP header can carry".into()))
}

/// How each call shows who calls.
#[derive(Clone)]
pub enum Auth {
    Signed(Arc<Signer>),
    Token(MetadataValue<tonic::metadata::Ascii>),
    None,
}

impl Interceptor for Auth {
    fn call(&mut self, mut request: tonic::Request<()>) -> std::result::Result<tonic::Request<()>, tonic::Status> {
        let value = match self {
            Self::Signed(signer) => signer.bearer().map_err(|error| tonic::Status::unauthenticated(error.to_string()))?,
            Self::Token(value) => value.clone(),
            Self::None => return Ok(request),
        };
        request.metadata_mut().insert("authorization", value);
        Ok(request)
    }
}

fn url_safe(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs()
}

/// An ES256 key of this program's, which signs its tokens.
pub struct Signer {
    key: EcdsaKeyPair,
    id: String,
    random: SystemRandom,
    token: Mutex<Option<(MetadataValue<tonic::metadata::Ascii>, Instant)>>,
}

impl Signer {
    fn new() -> Result<Self> {
        let random = SystemRandom::new();
        let failed = || DriverError::Failed("no key could be made for the emulator's interface".into());
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &random).map_err(|_| failed())?;
        let key = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref()).map_err(|_| failed())?;
        let mut id = [0u8; 16];
        aws_lc_rs::rand::fill(&mut id).map_err(|_| failed())?;
        Ok(Self {
            key,
            id: format!("demi-{}", id.iter().map(|byte| format!("{byte:02x}")).collect::<String>()),
            random,
            token: Mutex::default(),
        })
    }

    /// The public key as the emulator reads it: a JWK set of one.
    fn jwk_set(&self) -> String {
        // An uncompressed P-256 point: 0x04, then x and y, 32 bytes each.
        let point = self.key.public_key().as_ref();
        serde_json::json!({
            "keys": [{
                "kty": "EC",
                "crv": "P-256",
                "alg": "ES256",
                "use": "sig",
                "key_ops": ["verify"],
                "kid": self.id,
                "x": url_safe(&point[1..33]),
                "y": url_safe(&point[33..65]),
            }]
        })
        .to_string()
    }

    /// Puts the public key into the emulator's key directory and waits until
    /// the emulator lists it among its active keys.
    async fn register(&self, jwks: &Path, active: &Path) -> Result<Registered> {
        use notify::Watcher as _;
        let (changed, mut heard) = tokio::sync::mpsc::unbounded_channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            // The waiting end is gone once the key is taken or refused.
            let _ = changed.send(event);
        })
        .map_err(|error| DriverError::Failed(format!("the emulator's keys cannot be watched: {error}")))?;
        // The emulator rewrites the list of its active keys as it takes one.
        let listing = active.parent().unwrap_or(jwks);
        // Its directory tells of the file replaced, the file of its
        // contents rewritten in place.
        for watched in [listing, active] {
            watcher
                .watch(watched, notify::RecursiveMode::NonRecursive)
                .map_err(|error| DriverError::Failed(format!("{}: {error}", watched.display())))?;
        }
        let file = jwks.join(format!("{}.jwk", self.id));
        // Written whole beside the directory, then moved in: the emulator
        // reads each file as it appears there and deletes one it cannot
        // read.
        let partial = jwks.parent().unwrap_or(jwks).join(format!("{}.part", self.id));
        let written = async {
            tokio::fs::write(&partial, self.jwk_set()).await?;
            tokio::fs::rename(&partial, &file).await
        };
        written
            .await
            .map_err(|error| DriverError::Failed(format!("the key could not be given to the emulator: {error}")))?;
        let registered = Registered(file);
        let taken = async {
            loop {
                let listed = tokio::fs::read_to_string(active).await.unwrap_or_default();
                if listed.contains(&self.id) {
                    return true;
                }
                if heard.recv().await.is_none() {
                    return false;
                }
            }
        };
        match tokio::time::timeout(KEY_TAKEN, taken).await {
            Ok(true) => Ok(registered),
            _ => Err(DriverError::Failed(
                "the emulator did not take Demi's key for its gRPC interface".into(),
            )),
        }
    }

    /// The header of a token signed now or in the last [`TOKEN_RENEWAL`].
    fn bearer(&self) -> Result<MetadataValue<tonic::metadata::Ascii>> {
        let mut token = self.token.lock().expect("the token is intact");
        if let Some((value, signed)) = &*token
            && signed.elapsed() < TOKEN_RENEWAL
        {
            return Ok(value.clone());
        }
        // The emulator's validator refuses a `typ` header.
        let header = url_safe(serde_json::json!({ "alg": "ES256", "kid": self.id }).to_string().as_bytes());
        let issued = now();
        let claims = url_safe(
            serde_json::json!({ "iss": ISSUER, "iat": issued, "exp": issued + TOKEN_LIFETIME.as_secs() })
                .to_string()
                .as_bytes(),
        );
        let signed = format!("{header}.{claims}");
        let signature = self
            .key
            .sign(&self.random, signed.as_bytes())
            .map_err(|_| DriverError::Failed("the emulator's token could not be signed".into()))?;
        let value = bearer(&format!("{signed}.{}", url_safe(signature.as_ref())))?;
        *token = Some((value.clone(), Instant::now()));
        Ok(value)
    }
}

/// The key file this program put into an emulator's key directory, removed
/// as the connection goes.
struct Registered(PathBuf);

impl Drop for Registered {
    fn drop(&mut self) {
        // An emulator that ended took its directory with it.
        if let Err(error) = std::fs::remove_file(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::info!("{} was not removed: {error}", self.0.display());
        }
    }
}
