//! Startup and shutdown (`backend.md` § Startup and shutdown,
//! § Configuration).

use std::process::{Command, Output};

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
    ("DEMI_MACHINES_SOCKET", "/nonexistent/demi-machines.sock"),
    ("DEMI_NATIVE_CONFIG", "/nonexistent/native.json"),
];

fn refused_naming(output: &Output, variable: &str) {
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success(), "{stderr}");
    assert!(stderr.contains(variable), "{stderr}");
}

#[test]
fn a_port_that_is_not_a_number_stops_startup_naming_the_variable() {
    let mut variables = REQUIRED.to_vec();
    variables.push(("DEMI_BACKEND_PORT", "abc"));
    refused_naming(&start(&variables), "DEMI_BACKEND_PORT");
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
    let data = tempfile::Builder::new().prefix("demi-backend-").tempdir().unwrap();
    let mut variables = REQUIRED.to_vec();
    let data_dir = data.path().to_str().unwrap().to_owned();
    variables.push(("DEMI_BACKEND_DATA", &data_dir));
    variables.push(("DEMI_INSTANCE_SECRET", "not-a-hex-secret-value"));
    let output = start(&variables);
    refused_naming(&output, "DEMI_INSTANCE_SECRET");
    assert!(!String::from_utf8_lossy(&output.stderr).contains("not-a-hex-secret-value"));
}
