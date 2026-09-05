//! Module resolution and loading.
//!
//! Three kinds of specifier exist: `tinyjs:*` built-ins, `/embedded/*`
//! names for the bundle, and absolute file paths for runtime command
//! modules. Relative specifiers resolve against the importer. There is no
//! npm-style resolution of any kind.
//!
//! `tinyjs:*` and `/embedded/*` resolve only when the importer is the
//! embedded bundle; that check is what keeps the tinyjs API private to
//! the runner's machine layer — a module loaded from a file cannot reach the
//! bundle's own modules (already declared, and served from the module cache
//! if the name resolved) any more than the primitives.

use rquickjs::loader::{Loader, Resolver};
use rquickjs::module::Declared;
use rquickjs::{Ctx, Error, Module, Result};

use crate::payload::{Entry, Payload};

pub const SCHEME: &str = "tinyjs:";
const EMBEDDED_PREFIX: &str = "/embedded/";

pub struct ShellResolver;

impl Resolver for ShellResolver {
    fn resolve<'js>(&mut self, _ctx: &Ctx<'js>, base: &str, name: &str) -> Result<String> {
        if let Some(module) = name.strip_prefix(SCHEME) {
            if !base.starts_with(EMBEDDED_PREFIX) {
                return Err(Error::new_resolving_message(
                    base,
                    name,
                    "tinyjs modules are not available to modules loaded from files",
                ));
            }
            return match module {
                "fs" | "process" | "net" | "bytes" | "runtime" => Ok(name.to_string()),
                _ => Err(Error::new_resolving(base, name)),
            };
        }
        let resolved = if name.starts_with('/') {
            normalize(name)
        } else if name.starts_with("./") || name.starts_with("../") {
            let dir = match base.rfind('/') {
                Some(i) => &base[..i],
                None => "",
            };
            normalize(&format!("{dir}/{name}"))
        } else {
            return Err(Error::new_resolving_message(base, name, "only absolute and relative paths resolve"));
        };
        if resolved.starts_with(EMBEDDED_PREFIX) && !base.starts_with(EMBEDDED_PREFIX) {
            return Err(Error::new_resolving_message(base, name, "the embedded bundle is not available to modules loaded from files"));
        }
        Ok(resolved)
    }
}

/// Collapses `.` and `..` segments of an absolute path without touching the
/// file system.
fn normalize(path: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    for seg in path.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            s => out.push(s),
        }
    }
    format!("/{}", out.join("/"))
}

pub struct ShellLoader {
    payload: Payload,
}

impl ShellLoader {
    pub fn new(payload: Payload) -> Self {
        ShellLoader { payload }
    }
}

/// Evaluates the entry module under its `/embedded/` name: packed bytecode
/// is loaded as is, a file is compiled.
pub fn evaluate_entry<'js>(ctx: &Ctx<'js>, name: &str, entry: Entry) -> Result<rquickjs::Promise<'js>> {
    let module = match entry {
        // SAFETY: the bytecode was produced by this interpreter build from
        // the bundle at pack time and lives in the executable's own image.
        Entry::Bytecode(bytes) => unsafe { Module::load(ctx.clone(), bytes)? },
        Entry::Source(source) => Module::declare(ctx.clone(), name, source)?,
    };
    module.meta()?.set("url", file_url(name))?;
    let (_, promise) = module.eval()?;
    Ok(promise)
}

impl Loader for ShellLoader {
    fn load<'js>(&mut self, ctx: &Ctx<'js>, name: &str) -> Result<Module<'js, Declared>> {
        if let Some(module) = name.strip_prefix(SCHEME) {
            return match module {
                "fs" => Module::declare_def::<crate::fs::FsModule, _>(ctx.clone(), name),
                "process" => Module::declare_def::<crate::process::ProcessModule, _>(ctx.clone(), name),
                "net" => Module::declare_def::<crate::net::NetModule, _>(ctx.clone(), name),
                "bytes" => Module::declare_def::<crate::bytes::BytesModule, _>(ctx.clone(), name),
                "runtime" => Module::declare_def::<crate::runtime::RuntimeModule, _>(ctx.clone(), name),
                _ => Err(Error::new_loading(name)),
            };
        }
        if name.starts_with(EMBEDDED_PREFIX) {
            return match self.payload.source(name) {
                Some(Ok(source)) => declare_with_meta(ctx, name, source),
                Some(Err(e)) => Err(Error::new_loading_message(name, e.to_string())),
                None => Err(Error::new_loading(name)),
            };
        }
        let source = std::fs::read(name)
            .map_err(|e| Error::new_loading_message(name, e.to_string()))?;
        declare_with_meta(ctx, name, source)
    }
}

/// Declares a source module with `import.meta.url` set to its `file:` URL.
fn declare_with_meta<'js, S: Into<Vec<u8>>>(ctx: &Ctx<'js>, name: &str, source: S) -> Result<Module<'js, Declared>> {
    let module = Module::declare(ctx.clone(), name, source)?;
    module.meta()?.set("url", file_url(name))?;
    Ok(module)
}

/// `import.meta.url`: module names are absolute paths (`/embedded/…` for the
/// bundle), so the URL is `file://` plus the name.
fn file_url(name: &str) -> String {
    format!("file://{name}")
}
