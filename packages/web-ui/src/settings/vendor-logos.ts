/**
 * The vendors' marks, monochrome SVGs that `VendorMark` paints in the text's
 * color, found by file name: `logos/<vendor>.svg`. `bun xtask logos` keeps
 * models.dev's for every vendor the backend offers; the others are kept by
 * hand, their sources in `logos/hand-kept.txt`. A vendor without a mark shows
 * its initial.
 *
 * Each mark is a file of its own that the browser loads when a page shows
 * it: inlined, the two hundred marks would add half a megabyte to the bundle.
 */
const MARKS = import.meta.glob<string>('./logos/*.svg', {
  eager: true,
  query: '?url&no-inline',
  import: 'default',
})

/**
 * The vendor whose mark another wears: each subscription its vendor's,
 * Claude Code Claude's own, and each speech vendor its company's.
 */
const WORN: Record<string, string> = {
  'claude-code': 'claude',
  codex: 'openai',
  'grok-build': 'xai',
  'byteplus-speech': 'byteplus',
  'doubao-speech': 'volcengine',
}

/** The mark of `id` itself, by its file name; null when there is none. */
function mark(id: string): string | null {
  return MARKS[`./logos/${id}.svg`] ?? null
}

/** The mark of vendor `vendorId`; null when Demi has none for it. */
export function vendorLogo(vendorId: string | null): string | null {
  return vendorId ? mark(WORN[vendorId] ?? vendorId) : null
}

/**
 * The mark of a provider entry: a subscription's by its provider type, an
 * API key's by its vendor; null for a bare endpoint or a vendor Demi has no
 * mark for.
 */
export function providerLogo(entry: {
  kind: 'api_key' | 'subscription'
  providerType: string
  vendorId: string | null
}): string | null {
  return entry.kind === 'subscription'
    ? vendorLogo(entry.providerType)
    : vendorLogo(entry.vendorId)
}
