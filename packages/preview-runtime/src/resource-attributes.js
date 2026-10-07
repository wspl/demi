import { currentRewriter, noteLabels } from './url-map.js';

// Resource attributes in the DOM, by the Rust rewriter (crates/preview-rewrite/src/attributes.rs).

// options: { reflect } converts back for reading, { role } for frames and modules.
export function rewriteResourceAttribute(name, value, base, options = {}) {
  const result = currentRewriter().rewriteResourceAttribute(`${name}`, `${value}`, base, Boolean(options.reflect), options.role ?? 'resource');
  noteLabels();
  return result;
}

export function rewriteSrcset(value, base, options = {}) {
  const result = currentRewriter().rewriteSrcset(`${value}`, base, Boolean(options.reflect));
  noteLabels();
  return result;
}
