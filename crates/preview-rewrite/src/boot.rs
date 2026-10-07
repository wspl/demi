//! What a realm's runtime starts from: the boot data of a document or worker, and the code a
//! worker runs before its own script to install the runtime (`docs/browser/preview.md`
//! § Storage and browser features).

use serde_json::json;

use crate::address::Context;

/// Boot data for a document or worker whose runtime starts in the page: the context's
/// addresses and the labels it has met, with the realm's own URL, base and referrer.
pub fn boot_data(context: &Context, url: &str, base: &str, referrer: &str) -> serde_json::Value {
    // The realm's own label comes first: its runtime reads its own address through it.
    context.preview_origin(&context.document);
    let mut boot = serde_json::to_value(context).unwrap_or_default();
    boot["url"] = json!(url);
    boot["base"] = json!(base);
    boot["referrer"] = json!(referrer);
    boot["labels"] = json!(context.labels());
    boot
}

/// A worker's script as the runtime needs it: classic or module, and the address of the
/// document that started it.
#[derive(Clone, Copy, Debug)]
pub struct WorkerScript<'w> {
    pub module: bool,
    pub referrer: &'w str,
}

/// The channel to the Demi page, which the starting document sends as its first message (on
/// the worker, or on its port of a shared worker); the page's own code never sees it. A
/// shared worker's port starts at once, so that the message arrives.
const CHANNEL: &str = "globalThis.__demiPreview=(()=>{let resolve;const channel=new Promise(r=>{resolve=r;});\
const take=(target,capture)=>target.addEventListener('message',event=>{if(event.data?.__demiChannel!==1)return;event.stopImmediatePropagation();resolve(event.ports[0]);},capture);\
if(typeof SharedWorkerGlobalScope==='function'&&self instanceof SharedWorkerGlobalScope)addEventListener('connect',event=>{take(event.ports[0],false);event.ports[0].start();},true);\
else take(self,true);return Object.freeze({channel:()=>channel});})();";

/// One line a worker's script starts with: its boot data and channel, then the runtime. A
/// module imports them, since its own imports run before its body.
pub fn worker_prelude(context: &Context, url: &str, worker: WorkerScript) -> String {
    let runtime = format!("{}{}", context.preview_origin(&context.document), context.runtime);
    let setup = format!("globalThis.__proxyBoot={};{CHANNEL}", boot_data(context, url, url, worker.referrer));
    if worker.module {
        let encoded = percent_encoding::utf8_percent_encode(&setup, percent_encoding::NON_ALPHANUMERIC);
        format!("import \"data:text/javascript,{encoded}\";import {};", json!(runtime))
    } else {
        format!("{setup}importScripts({});", json!(runtime))
    }
}
