//! Startup and shutdown (`backend.md` § Startup and shutdown,
//! § Configuration).

use std::net::{Ipv4Addr, SocketAddr};
use std::os::unix::fs::PermissionsExt as _;
use std::process::{Command, Output};

use demi_backend::{Backend, BackendConfig, InstanceSecret, SecretError, StartError};
use demi_web_api_protocol::settings::InstanceMode;

use crate::support::Harness;

#[tokio::test]
async fn shutdown_closes_the_listener() {
    let harness = Harness::new();
    let backend = harness.start().await;
    let url = backend.url.clone();
    backend.close().await;
    let after = reqwest::Client::builder()
        .no_proxy()
        .build()
        .unwrap()
        .get(format!("{url}/api/setup"))
        .send()
        .await;
    assert!(after.is_err(), "the backend still answers after shutdown");
}

/// The binary started with exactly these variables.
fn start(variables: &[(&str, &str)]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_demi-backend"))
        .env_clear()
        .envs(variables.iter().copied())
        .output()
        .unwrap()
}

const REQUIRED: [(&str, &str); 4] = [
    ("DEMI_INSTANCE_MODE", "shared"),
    ("DEMI_BACKEND_PUBLIC_URL", "http://127.0.0.1:3271"),
    (
        "DEMI_MACHINE_MANAGER_SOCKET",
        "/nonexistent/demi-machine-manager.sock",
    ),
    ("DEMI_RELEASE", "/nonexistent/release"),
];

fn refused_naming(output: &Output, variable: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(stderr.contains(variable), "{stderr}");
}

#[test]
fn a_listening_address_without_a_port_stops_startup_naming_the_variable() {
    let mut variables = REQUIRED.to_vec();
    variables.push(("DEMI_BACKEND_LISTEN", "127.0.0.1"));
    refused_naming(&start(&variables), "DEMI_BACKEND_LISTEN");
}

#[test]
fn a_variable_the_backend_does_not_read_stops_startup_naming_it() {
    let mut variables = REQUIRED.to_vec();
    variables.push(("DEMI_BACKEND_LISTENN", "127.0.0.1:3272"));
    refused_naming(&start(&variables), "DEMI_BACKEND_LISTENN");
}

/// The backend and the machine manager read one configuration file: the
/// manager's own settings are the manager's to check, so the backend goes
/// on to the next step, here the release root that does not exist.
#[test]
fn the_machine_managers_settings_pass_the_backends_check() {
    let mut variables = REQUIRED.to_vec();
    variables.push(("DEMI_MANAGED_SLOTS", "16"));
    let output = start(&variables);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(!stderr.contains("DEMI_MANAGED_SLOTS"), "{stderr}");
    assert!(stderr.contains("/nonexistent/release/release.json"), "{stderr}");
}

#[test]
fn s3_settings_without_an_s3_store_stop_startup_naming_them() {
    let mut variables = REQUIRED.to_vec();
    variables.push(("DEMI_S3_BUCKET", "demi"));
    refused_naming(&start(&variables), "DEMI_S3_BUCKET");
    let mut variables = REQUIRED.to_vec();
    variables.push(("DEMI_STORAGE", "s3"));
    variables.push(("DEMI_S3_BUCKET", "demi"));
    refused_naming(&start(&variables), "DEMI_S3_REGION");
}

/// `DEMI_STUN_URLS` names `stun:` URLs, or nothing, which keeps direct
/// channels to one network: an empty value passes the check, and the start
/// goes on to the next step, the release root that does not exist.
#[test]
fn stun_urls_are_stun_urls_or_nothing() {
    let mut variables = REQUIRED.to_vec();
    variables.push(("DEMI_STUN_URLS", "https://stun.example.com"));
    refused_naming(&start(&variables), "DEMI_STUN_URLS");
    for passing in ["", "stun:stun.example.com,stun:[2001:db8::1]:3479"] {
        let mut variables = REQUIRED.to_vec();
        variables.push(("DEMI_STUN_URLS", passing));
        let output = start(&variables);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(!stderr.contains("DEMI_STUN_URLS"), "{passing:?}: {stderr}");
        assert!(stderr.contains("/nonexistent/release/release.json"), "{stderr}");
    }
}

#[test]
fn a_missing_public_url_stops_startup_naming_the_variable() {
    let variables: Vec<_> = REQUIRED
        .into_iter()
        .filter(|(name, _)| *name != "DEMI_BACKEND_PUBLIC_URL")
        .collect();
    refused_naming(&start(&variables), "DEMI_BACKEND_PUBLIC_URL");
}

#[test]
fn a_malformed_instance_secret_stops_startup_without_showing_it() {
    let data = tempfile::Builder::new()
        .prefix("demi-backend-")
        .tempdir()
        .unwrap();
    let mut variables = REQUIRED.to_vec();
    let data_dir = data.path().to_str().unwrap().to_owned();
    variables.push(("DEMI_BACKEND_DATA", &data_dir));
    variables.push(("DEMI_INSTANCE_SECRET", "not-a-hex-secret-value"));
    let output = start(&variables);
    refused_naming(&output, "DEMI_INSTANCE_SECRET");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("not-a-hex-secret-value"));
}

/// The data directory keeps the instance secret the vault's and the email
/// codes' keys derive from: the first start creates it readable by its owner
/// only, a later start reads it again, and a corrupt one stops startup.
#[tokio::test]
async fn the_instance_secret_is_created_once_readable_by_its_owner_only() {
    let harness = Harness::new();
    harness.start().await.close().await;
    let path = harness.data_dir().join("instance-secret");
    let created = std::fs::read_to_string(&path).unwrap();
    let mode = std::fs::metadata(&path).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o600);
    harness.start().await.close().await;
    assert_eq!(std::fs::read_to_string(&path).unwrap(), created);

    std::fs::write(&path, "not hex\n").unwrap();
    let config = BackendConfig::new(
        harness.data_dir(),
        SocketAddr::from((Ipv4Addr::LOCALHOST, 0)),
        InstanceMode::Shared,
        "/nonexistent/demi-machine-manager.sock".into(),
    );
    let Err(refused) = Backend::start(config).await else {
        panic!("a corrupt instance secret started the backend");
    };
    assert!(
        matches!(refused, StartError::Secret(SecretError::Corrupt { .. })),
        "{refused}"
    );
}

#[test]
fn a_configured_secret_is_64_hex_digits() {
    let digits = "0123456789abcdef".repeat(4);
    assert!(digits.parse::<InstanceSecret>().is_ok());
    assert!(digits.to_uppercase().parse::<InstanceSecret>().is_ok());
    assert!(digits[1..].parse::<InstanceSecret>().is_err());
    assert!(
        format!("{}zz", &digits[2..])
            .parse::<InstanceSecret>()
            .is_err()
    );
}
