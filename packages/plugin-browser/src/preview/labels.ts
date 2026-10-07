/**
 * Preview addresses as the relay reads them (`preview.md` § Addresses and
 * labels). The relay never computes a label: the engine does, for the
 * address the user opens, the addresses its answers map, and the
 * environments a runtime registers. Here the relay only reads a label out
 * of a preview origin, and a real address out of a preview address.
 */
import type { PreviewPlace } from '@demicodes/plugin-sdk'
import { PREVIEW_FILES_VERSION, type PreviewEnvironment } from '../generated/plugin'

/** The boot page's path, which every preview origin serves. */
export const BOOT_PATH = `/__demi/v${PREVIEW_FILES_VERSION}/boot.html`

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
