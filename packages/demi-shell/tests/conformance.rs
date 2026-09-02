//! Runs the primitive conformance suite: the binary built with the
//! `conformance` feature embeds the suite as its bundle. This test
//! provisions what the suite cannot make itself — free ports, a test CA and
//! server certificate (openssl), the path to Bun for the stub — and passes
//! them through the environment.

use std::net::TcpListener;
use std::path::PathBuf;
use std::process::Command;

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
}

fn which(name: &str) -> Option<PathBuf> {
    let out = Command::new("which").arg(name).output().ok()?;
    if !out.status.success() {
        return None;
    }
    Some(PathBuf::from(String::from_utf8_lossy(&out.stdout).trim()))
}

/// Generates a CA and a server certificate for localhost / 127.0.0.1.
fn make_certs(dir: &PathBuf) -> Option<(PathBuf, PathBuf, PathBuf)> {
    let openssl = which("openssl")?;
    let ca_key = dir.join("ca.key");
    let ca = dir.join("ca.pem");
    let key = dir.join("server.key");
    let csr = dir.join("server.csr");
    let cert = dir.join("server.pem");
    let ext = dir.join("san.cnf");
    std::fs::write(&ext, "subjectAltName=DNS:localhost,IP:127.0.0.1\n").ok()?;
    let run = |args: &[&str]| Command::new(&openssl).args(args).output().map(|o| o.status.success()).unwrap_or(false);
    let ok = run(&["req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-keyout", ca_key.to_str()?, "-out", ca.to_str()?, "-days", "2", "-subj", "/CN=demi-shell test CA"])
        && run(&["req", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-keyout", key.to_str()?, "-out", csr.to_str()?, "-subj", "/CN=localhost"])
        && run(&["x509", "-req", "-in", csr.to_str()?, "-CA", ca.to_str()?, "-CAkey", ca_key.to_str()?, "-CAcreateserial", "-out", cert.to_str()?, "-days", "2", "-extfile", ext.to_str()?]);
    ok.then_some((ca, cert, key))
}

/// Bundles the runner-protocol codec the way the runner bundle will be
/// built, so the suite can prove it runs on the shell.
fn bundle_runner_protocol(bun: &PathBuf, work: &PathBuf) -> Option<PathBuf> {
    let entry = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../runner-protocol/src/messages.ts");
    let out = work.join("runner-protocol.mjs");
    let ok = Command::new(bun)
        .args(["build", entry.to_str()?, "--format=esm", "--target=browser", "--outfile"])
        .arg(&out)
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);
    ok.then_some(out)
}

#[test]
#[cfg_attr(not(feature = "conformance"), ignore = "build with --features conformance")]
fn primitive_conformance_suite() {
    let bin = env!("CARGO_BIN_EXE_demi-shell");
    let conformance_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("conformance");
    let work = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("conformance");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();

    let mut cmd = Command::new(bin);
    cmd.env_remove("HTTPS_PROXY").env_remove("HTTP_PROXY").env_remove("NO_PROXY");
    cmd.env_remove("https_proxy").env_remove("http_proxy").env_remove("no_proxy");
    if let Some(bun) = which("bun") {
        if let Some(bundle) = bundle_runner_protocol(&bun, &work) {
            cmd.env("DEMI_SHELL_CONFORMANCE_PROTOCOL", bundle);
        }
        let ports = [free_port(), free_port(), free_port()];
        cmd.env("DEMI_SHELL_CONFORMANCE_DIR", &conformance_dir)
            .env("DEMI_SHELL_CONFORMANCE_BUN", &bun)
            .env("DEMI_SHELL_CONFORMANCE_PORTS", format!("{},{},{}", ports[0], ports[1], ports[2]))
            .env("HTTPS_PROXY", format!("http://user:pass@127.0.0.1:{}", ports[2]));
        if let Some((ca, cert, key)) = make_certs(&work) {
            cmd.env("DEMI_SHELL_CONFORMANCE_CA", ca)
                .env("DEMI_SHELL_CONFORMANCE_CERT", cert)
                .env("DEMI_SHELL_CONFORMANCE_KEY", key);
        } else {
            eprintln!("openssl not available: TLS cases will be skipped");
        }
    } else {
        eprintln!("bun not available: stub-backed net cases will be skipped");
    }
    let status = cmd.status().expect("run the shell");
    assert!(status.success(), "conformance suite failed: {status}");
}
