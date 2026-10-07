//! The rewriter's inputs and outputs (`docs/browser/preview.md` § Addresses
//! and labels, § Rewriting): what the engine and the runtime both rely on.
//! Pure functions over strings; the whole binary runs in well under a second.

mod address;
mod css;
mod html;
mod javascript;

use demi_preview_rewrite::address::{Context, Environment, label, site_of};

const DOMAIN: &str = "demi-preview.test";
const NAMESPACE: &str = "k3f9x2ab";
const HOST: &str = "host-1";
const BOOT: &str = "/__demi/v1/boot.html";
const RUNTIME: &str = "/__demi/page/runtime/0.1.0.js";

/// The context of a preview tab's top document at `origin`.
fn top_document(origin: &str) -> Context {
    Context {
        scheme: "https".into(),
        domain: DOMAIN.into(),
        namespace: NAMESPACE.into(),
        host: HOST.into(),
        boot: BOOT.into(),
        client: "/__demi/v1/client.js".into(),
        runtime: RUNTIME.into(),
        document: Environment { origin: origin.into(), top: site_of(origin), cross: false },
        top_level: true,
        opaque: false,
        labels: Default::default(),
    }
}

/// The preview origin of an environment, computed independently of a context.
fn preview_origin(origin: &str, top: &str, cross: bool) -> String {
    let environment = Environment { origin: origin.into(), top: top.into(), cross };
    format!("https://{NAMESPACE}--{}.{DOMAIN}", label(NAMESPACE, HOST, &environment))
}
