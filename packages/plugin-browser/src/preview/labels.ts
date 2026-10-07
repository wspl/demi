/**
 * Labels and preview addresses as the relay reads them (`preview.md`
 * § Addresses and labels). The label algorithm is public and never changes
 * between releases: the first 80 bits of the SHA-256 of (namespace, Host,
 * logical origin, top-level site, cross-site ancestor), in lowercase
 * base32hex. The rewriter computes it in Rust (`preview-rewrite`), and the
 * relay computes it again here, with the browser's own SHA-256, before it
 * keeps any label it is told of: a label this page keeps always names the
 * environment it was computed from. Both are pinned by the same value.
 */
import type { PreviewPlace } from '@demicodes/plugin-sdk'
import type { PreviewEnvironment } from '../generated/plugin'

const ALPHABET = '0123456789abcdefghijklmnopqrstuv'
const LABEL_BYTES = 10
const encoder = new TextEncoder()

/** `bytes` in lowercase base32hex, without padding. */
function base32hex(bytes: Uint8Array): string {
  let bits = 0
  let value = 0
  let output = ''
  for (const byte of bytes) {
    value = ((value << 8) | byte) & 0xffff
    bits += 8
    while (bits >= 5) {
      output += ALPHABET[(value >>> (bits - 5)) & 31]
      bits -= 5
    }
  }
  return bits > 0 ? output + ALPHABET[(value << (5 - bits)) & 31] : output
}

/** The label of `environment` in `place`'s namespace and Host. */
export async function labelOf(place: Pick<PreviewPlace, 'namespace' | 'host'>, environment: PreviewEnvironment): Promise<string> {
  const text = `${place.namespace}\n${place.host}\n${environment.origin}\n${environment.top}\n${environment.cross ? 1 : 0}`
  const digest = new Uint8Array(await crypto.subtle.digest('SHA-256', encoder.encode(text)))
  return base32hex(digest.subarray(0, LABEL_BYTES))
}

/** The preview origin of `label`: `<scheme>://<namespace>--<label>.<domain>`. */
export function previewOrigin(place: PreviewPlace, label: string): string {
  return `${place.scheme}://${place.namespace}--${label}.${place.domain}`
}

/** The boot page's path, which every preview origin serves. */
export const BOOT_PATH = '/__demi/v1/boot.html'

/**
 * The label of a preview origin of `place`'s namespace, or null for any
 * other origin. A page may also build one from the preview host alone,
 * without the port, which names the same origin where the domain is served
 * on the scheme's default port.
 */
export function labelOfOrigin(place: PreviewPlace, origin: string): string | null {
  const url = URL.parse(origin)
  if (!url || url.protocol !== `${place.scheme}:`) {
    return null
  }
  const [domainHost, domainPort = ''] = place.domain.split(':')
  const match = /^([a-z0-9]+)--([0-9a-v]{16})\.(.+)$/.exec(url.hostname)
  if (!match || match[1] !== place.namespace || match[3] !== domainHost) {
    return null
  }
  if (url.port !== domainPort && url.port !== '') {
    return null
  }
  return match[2]!
}

/**
 * The real address behind a preview address of `environment`: its path,
 * query and fragment on the environment's origin; a boot page's address
 * leads to the target in its fragment. The engine's `__demi_` parameters
 * stay, for the engine reads them.
 */
export function realAddress(preview: URL, environment: PreviewEnvironment): string {
  const target = preview.pathname === BOOT_PATH
    ? (/(?:^#|&)to=(.*)$/.exec(preview.hash)?.[1] ?? '/')
    : preview.pathname + preview.search + preview.hash
  return environment.origin + target
}
