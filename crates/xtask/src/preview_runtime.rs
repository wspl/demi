//! `xtask preview-runtime` (`builds-and-releases.md` § Preview runtime):
//! builds `preview-rewrite-wasm` for WebAssembly in the release profile,
//! generates its JavaScript glue into `@demicodes/preview-runtime`'s
//! `src/generated/`, and bundles the runtime with the module inlined into
//! `packages/preview-runtime/dist/runtime/<release>.js`, the directory the web
//! app's build carries.

use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

/// The runtime's package, relative to the repository root.
const PACKAGE: &str = "packages/preview-runtime";
/// The WebAssembly target the rewriter is built for.
const TARGET: &str = "wasm32-unknown-unknown";
/// The rewriter's crate, and the module Cargo names after it.
const CRATE: &str = "demi-preview-rewrite-wasm";
const MODULE: &str = "demi_preview_rewrite_wasm.wasm";
/// The stem of the generated files, which `src/rewriter.js` imports.
const GLUE: &str = "preview_rewrite_wasm";
const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(clap::Args)]
pub struct Options {
    /// The release the runtime is named by [default: the workspace version].
    #[arg(long, value_name = "VERSION", value_parser = release)]
    release: Option<String>,
    /// Keeps names and adds an inline source map, for reading stacks in a
    /// page.
    #[arg(long)]
    debug: bool,
}

/// A release is named by its version, which becomes a file name.
fn release(value: &str) -> Result<String, semver::Error> {
    Ok(semver::Version::parse(value)?.to_string())
}

/// Why building stopped.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{step} failed: {status}")]
    Step { step: &'static str, status: ExitStatus },
    #[error("generating the WebAssembly glue failed: {0}")]
    Glue(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

pub fn run(options: Options) -> Result<(), Error> {
    let repository = crate::repository();
    let package = repository.join(PACKAGE);
    let module = build_module(&repository)?;
    let generated = package.join("src/generated");
    wasm_bindgen_cli_support::Bindgen::new()
        .input_path(&module)
        .out_name(GLUE)
        .web(true)
        .and_then(|bindgen| bindgen.generate(&generated))
        .map_err(|error| Error::Glue(format!("{error:#}")))?;
    // The directory holds this build's runtime only, so the web app's build
    // carries no runtime of an earlier release.
    let directory = package.join("dist/runtime");
    match std::fs::remove_dir_all(&directory) {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return Err(error.into()),
        _ => {}
    }
    std::fs::create_dir_all(&directory)?;
    let release = options.release.unwrap_or_else(|| VERSION.to_owned());
    let output = directory.join(format!("{release}.js"));
    let mut bundle = Command::new("bun");
    bundle.arg(package.join("bundle.ts")).arg(&output).current_dir(&package);
    if options.debug {
        bundle.arg("--debug");
    }
    succeed("bundling the runtime", &mut bundle)?;
    let size = |path: &Path| std::fs::metadata(path).map(|metadata| metadata.len());
    println!(
        "wrote {} ({} bytes, WebAssembly {} bytes)",
        output.display(),
        size(&output)?,
        size(&generated.join(format!("{GLUE}_bg.wasm")))?
    );
    Ok(())
}

/// Builds the rewriter's module and returns its path.
fn build_module(repository: &Path) -> Result<PathBuf, Error> {
    // `cargo run` names the toolchain's cargo; `bun xtask` takes the one on
    // PATH.
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let mut build = Command::new(cargo);
    build
        .args(["build", "--release", "--target", TARGET, "--package", CRATE])
        .current_dir(repository)
        // The workspace's profiles are the only compiler options
        // (`builds-and-releases.md` § Build profiles).
        .env_remove("RUSTFLAGS");
    succeed("building the rewriter's WebAssembly", &mut build)?;
    // A relative target directory is the repository's, where Cargo ran.
    let target = std::env::var_os("CARGO_TARGET_DIR")
        .map_or_else(|| repository.join("target"), |directory| repository.join(directory));
    Ok(target.join(TARGET).join("release").join(MODULE))
}

fn succeed(step: &'static str, command: &mut Command) -> Result<(), Error> {
    let status = command.status()?;
    if status.success() {
        Ok(())
    } else {
        Err(Error::Step { step, status })
    }
}
