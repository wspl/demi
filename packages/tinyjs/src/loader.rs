//! Module resolution and loading.
//!
//! Three kinds of specifier exist: `tinyjs:*` built-ins, `/embedded/*`
//! names for the bundle, and absolute file paths for runtime command
//! modules. Relative specifiers resolve against the importer. There is no
//! npm-style resolution of any kind.
//!
//! `tinyjs:*` resolves only when the importer is the embedded bundle;
//! that check is what keeps the tinyjs API private to `@demicodes/host-tinyjs`.

use rquickjs::loader::{Loader, Resolver};
use rquickjs::module::Declared;
use rquickjs::{Ctx, Error, Module, Result};

use crate::embedded;

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
        if name.starts_with('/') {
            return Ok(normalize(name));
        }
        if name.starts_with("./") || name.starts_with("../") {
            let dir = match base.rfind('/') {
                Some(i) => &base[..i],
                None => "",
            };
            return Ok(normalize(&format!("{dir}/{name}")));
        }
        Err(Error::new_resolving_message(base, name, "only absolute and relative paths resolve"))
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

pub struct ShellLoader;

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
        if let Some(source) = embedded::source(name) {
            return declare_with_meta(ctx, name, source);
        }
        if name.starts_with(EMBEDDED_PREFIX) {
            return Err(Error::new_loading(name));
        }
        let source = std::fs::read(name)
            .map_err(|e| Error::new_loading_message(name, e.to_string()))?;
        declare_with_meta(ctx, name, source)
    }
}

/// Declares a source module with `import.meta.url` set to its name.
fn declare_with_meta<'js, S: Into<Vec<u8>>>(ctx: &Ctx<'js>, name: &str, source: S) -> Result<Module<'js, Declared>> {
    let module = Module::declare(ctx.clone(), name, source)?;
    module.meta()?.set("url", name)?;
    Ok(module)
}
