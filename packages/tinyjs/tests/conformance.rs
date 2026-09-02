//! Runs the primitive conformance suite on the bare binary
//! (`tinyjs conformance/main.mjs`). This test provisions what the suite
//! cannot make itself — free ports, a test CA and
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
    let ok = run(&["req", "-x509", "-newkey", "ec", "-pkeyopt", "ec_paramgen_curve:prime256v1", "-nodes", "-keyout", ca_key.to_str()?, "-out", ca.to_str()?, "-days", "2", "-subj", "/CN=tinyjs test CA"])
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
fn primitive_conformance_suite() {
    let bin = env!("CARGO_BIN_EXE_tinyjs");
    let conformance_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("conformance");
    let work = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("conformance");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();

    let mut cmd = Command::new(bin);
    cmd.arg(conformance_dir.join("main.mjs"));
    cmd.env_remove("HTTPS_PROXY").env_remove("HTTP_PROXY").env_remove("NO_PROXY");
    cmd.env_remove("https_proxy").env_remove("http_proxy").env_remove("no_proxy");
    if let Some(bun) = which("bun") {
        if let Some(bundle) = bundle_runner_protocol(&bun, &work) {
            cmd.env("TINYJS_CONFORMANCE_PROTOCOL", bundle);
        }
        let ports = [free_port(), free_port(), free_port()];
        cmd.env("TINYJS_CONFORMANCE_DIR", &conformance_dir)
            .env("TINYJS_CONFORMANCE_BUN", &bun)
            .env("TINYJS_CONFORMANCE_PORTS", format!("{},{},{}", ports[0], ports[1], ports[2]))
            .env("HTTPS_PROXY", format!("http://user:pass@127.0.0.1:{}", ports[2]));
        if let Some((ca, cert, key)) = make_certs(&work) {
            cmd.env("TINYJS_CA_FILE", ca)
                .env("TINYJS_CONFORMANCE_CERT", cert)
                .env("TINYJS_CONFORMANCE_KEY", key);
        } else {
            eprintln!("openssl not available: TLS cases will be skipped");
        }
    } else {
        eprintln!("bun not available: stub-backed net cases will be skipped");
    }
    let status = cmd.status().expect("run tinyjs");
    assert!(status.success(), "conformance suite failed: {status}");
}

/// Packs a bundle with `tinyjs --pack` and checks that the packed file runs
/// it with `argv[0]` as the invoked name and `tinyjs:*` visible.
#[test]
fn packed_binary_runs_the_bundle() {
    let bin = env!("CARGO_BIN_EXE_tinyjs");
    let work = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("packed");
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).unwrap();
    let packed = work.join("tinyjs-packed");
    let bundle = work.join("bundle.mjs");
    std::fs::write(&bundle, r#"import { argv, exit } from "tinyjs:runtime";
import { sha256 } from "tinyjs:bytes";
const name = argv[0].slice(argv[0].lastIndexOf("/") + 1);
console.log(`${name}:${argv.slice(1).join(",")}:${sha256(new Uint8Array(0)).length}`);
exit(name === "demi-runner" ? 7 : 0);
"#).unwrap();
    let out = Command::new(bin).arg("--pack").arg(&bundle).arg("--out").arg(&packed).output().unwrap();
    assert!(out.status.success(), "pack failed: {}", String::from_utf8_lossy(&out.stderr));
    if cfg!(target_os = "macos") {
        let status = Command::new("codesign").args(["--verify", "--strict"]).arg(&packed).status().unwrap();
        assert!(status.success(), "the packed binary must pass strict signature verification");
    }
    let demi = work.join("demi");
    let runner = work.join("demi-runner");
    std::os::unix::fs::symlink(&packed, &demi).unwrap();
    std::os::unix::fs::symlink(&packed, &runner).unwrap();

    let out = Command::new(&demi).args(["a", "b"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "demi:a,b:32", "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(out.status.code(), Some(0));
    let out = Command::new(&runner).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "demi-runner::32");
    assert_eq!(out.status.code(), Some(7));
    // A packed binary parses no arguments: --pack goes to the bundle.
    let out = Command::new(&demi).args(["--pack"]).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "demi:--pack:32");

    // The bare binary without an entry prints usage and exits with 2.
    let out = Command::new(bin).output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("usage"));
}
