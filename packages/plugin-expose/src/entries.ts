import type { ExposeMenuEntry } from './types'
import type { ExposeEntry } from './generated/plugin'

/** The plugin's exposes as the menu lists them, in the state's order, each naming its host. */
export function menuEntries(exposes: readonly ExposeEntry[]): ExposeMenuEntry[] {
  return exposes.map((expose) => ({
    id: expose.id,
    address: expose.address,
    hostName: expose.deviceName,
    url: expose.url,
    expiresAt: expose.expiresAt,
  }))
}
