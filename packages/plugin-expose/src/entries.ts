import type { ExposeMenuEntry } from './types'
import type { ExposeEntry } from './generated/plugin'

/** The plugin's exposes as the menu lists them, in the state's order, each naming its host as the page does. */
export function menuEntries(
  exposes: readonly ExposeEntry[],
  hostName: (id: string) => string,
): ExposeMenuEntry[] {
  return exposes.map((expose) => ({
    id: expose.id,
    address: expose.address,
    hostName: hostName(expose.deviceId),
    url: expose.url,
    expiresAt: expose.expiresAt,
  }))
}
