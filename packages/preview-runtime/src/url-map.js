import { wasm } from './rewriter.js';

// Preview addresses: <scheme>://<namespace>--<label>.<preview domain>/<path>, the scheme the
// backend's (http for a development domain), where the label names a document environment
// (`docs/browser/preview.md` § Addresses and labels); the rules live in the
// Rust rewriter (crates/preview-rewrite/src/address.rs). A realm's rewriter starts from its boot data,
// and reports every new label it maps to the Demi page, which checks it.
let rewriter;
let reportLabels = () => {};
const reported = new Set();
let reportScheduled = false;

export function startRewriter(boot, report) {
  rewriter = new wasm.Rewriter(JSON.stringify(boot));
  for (const label of Object.keys(boot.labels ?? {})) reported.add(label);
  reportLabels = report;
}

// Use a rewriter for a realm this one starts from `boot` (a data: worker, whose source it
// rewrites); this realm learns the labels it maps, and reports them with its own.
export function withRewriter(boot, use) {
  const other = new wasm.Rewriter(JSON.stringify(boot));
  try {
    const result = use(other);
    currentRewriter().learn(other.labels());
    return result;
  } finally {
    other.free();
  }
}

export function currentRewriter() {
  if (!rewriter) throw new Error('No rewriter for this realm');
  return rewriter;
}

// Labels mapped since the last report, sent once the current task's mappings are done.
function scheduleReport() {
  if (reportScheduled) return;
  reportScheduled = true;
  queueMicrotask(() => {
    reportScheduled = false;
    const entries = Object.entries(JSON.parse(rewriter.labels())).filter(([label]) => !reported.has(label));
    if (!entries.length) return;
    for (const [label] of entries) reported.add(label);
    reportLabels(Object.fromEntries(entries));
  });
}

function mapped(value) {
  scheduleReport();
  return value;
}

export function isProxyUrl(value) {
  return currentRewriter().isPreviewUrl(String(value));
}

// Map an absolute or base-relative logical address to its preview address. Like the URL
// constructor, an address that does not parse is a TypeError.
export function mapUrl(value, base, role = 'resource') {
  const result = currentRewriter().mapUrl(String(value), base === undefined ? undefined : String(base), role);
  if (result === undefined) throw new TypeError(`Invalid URL: ${value}`);
  return mapped(result);
}

// The logical address behind a preview address; the address itself when its label is unknown.
export function logicalUrl(value) {
  return currentRewriter().logicalUrl(String(value)) ?? String(value);
}

// Map an address as written in markup or CSS: only absolute addresses change.
export function mapWrittenUrl(text, base, role = 'resource') {
  return mapped(currentRewriter().convertWrittenUrl(`${text}`, String(base), false, role));
}

// The logical address the page wrote: the reverse of mapWrittenUrl.
export function writtenUrl(text) {
  return currentRewriter().writtenUrl(`${text}`);
}

// How an element's address attribute is used: a frame, a module, a navigation, a resource.
export function addressRole(element, attribute) {
  const tag = element.localName ?? element.tagName?.toLowerCase();
  const read = name => element.getAttribute?.(name) ?? undefined;
  return wasm.addressRole(tag, attribute, read('type'), read('rel'), read('target'));
}

// A preview address whose response the engine checks against Subresource Integrity metadata.
export function withIntegrity(address, integrity) {
  return integrity ? wasm.withParameter(String(address), 'integrity', String(integrity)) : address;
}

// A worker's own script address: the engine starts the worker's runtime in front of it.
export function markWorker(address, module) {
  return wasm.withParameter(String(address), 'worker', module ? 'module' : 'classic');
}

export function noteLabels() {
  scheduleReport();
}
