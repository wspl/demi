import { currentRewriter, noteLabels } from './url-map.js';

// JavaScript and HTML rewriting by the Rust rewriter (`preview-rewrite`).

// options.sourceMap: 'omitted' (default), 'inline', 'returned' (gives { code, map }), or the
// URL a map is served at. options.base resolves dynamic imports; options.moduleUrl is what
// import.meta.url reads. Code that does not parse is a SyntaxError, as eval would report.
export function rewriteJavaScript(source, filename = 'upstream.js', options = {}) {
  const mode = options.sourceMap ?? 'omitted';
  let output;
  try {
    output = JSON.parse(currentRewriter().rewriteJavaScript(source, filename, options.base, options.moduleUrl, filename === 'inline-handler.js', mode));
  } catch (error) {
    throw new SyntaxError(error.message);
  }
  noteLabels();
  return mode === 'returned' ? output : output.code;
}

// The Function constructor's arguments, rewritten: [parameters, body].
export function rewriteFunctionArguments(args, kind) {
  const parts = args.map(String);
  const body = parts.pop() ?? '';
  try {
    return currentRewriter().rewriteFunction(parts.join(','), body, kind);
  } catch (error) {
    throw new SyntaxError(error.message);
  }
}

export function rewriteRefresh(value, base) {
  const result = currentRewriter().rewriteRefresh(String(value), base);
  noteLabels();
  return result;
}

// options.fragment rewrites markup inserted into a document; options.boot (boot data as
// JSON) starts the runtime in a document of its own.
export function rewriteHtml(source, documentUrl, options = {}) {
  const result = currentRewriter().rewriteHtml(String(source), documentUrl, Boolean(options.fragment), options.boot);
  noteLabels();
  return result;
}
