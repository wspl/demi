import { currentRewriter, noteLabels } from './url-map.js';

// Map CSS resource addresses and attribute selectors on URL attributes while preserving all
// unrelated source bytes; with options.reflect, back to the text the page wrote. Any CSS
// context: a stylesheet, a declaration list, a single value or a selector. Selectors match
// the document at options.documentBase (the base, unless a stylesheet has its own address).
export function rewriteCss(value, base, _context, options = {}) {
  const result = currentRewriter().rewriteCss(`${value}`, base, options.documentBase ?? base, Boolean(options.reflect));
  noteLabels();
  return result;
}
