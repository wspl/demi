use std::{env, path::PathBuf, process::Command};

fn generate_contracts(target: &str) {
    let crate_dir =
        PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    let root = crate_dir
        .parent()
        .and_then(|path| path.parent())
        .expect("repository root");
    for path in [
        "scripts/rust-build.rs",
        "scripts/rust-zod.ts",
        "scripts/generate-contracts.ts",
        "packages/runner-protocol/src",
        "packages/command-protocol/src",
        "packages/command-loader/src/manifest",
        "packages/shell/src",
        "packages/utils/src",
        "bun.lock",
        "package.json",
    ] {
        println!("cargo:rerun-if-changed={}", root.join(path).display());
    }
    let output = env::var_os("OUT_DIR").expect("Cargo output directory");
    let status = Command::new("bun")
        .arg("--conditions=development")
        .arg(root.join("scripts/generate-contracts.ts"))
        .arg(target)
        .arg(output)
        .current_dir(root)
        .status()
        .expect(
            "Cargo contract generation requires bun on PATH and installed workspace dependencies",
        );
    assert!(status.success(), "Zod-to-Rust contract generation failed");
}
