import { currentRewriter, noteLabels } from './url-map.js';

// Map absolute module specifiers to preview addresses; the browser resolves the rest.
export function mapModuleSpecifier(value, base) {
  const result = currentRewriter().mapModuleSpecifier(`${value}`, base);
  noteLabels();
  return result;
}

// The address-bearing parts of an import map; an invalid map stays as written.
export function rewriteImportMap(source, base) {
  const result = currentRewriter().rewriteImportMap(`${source}`, base);
  noteLabels();
  return result;
}
