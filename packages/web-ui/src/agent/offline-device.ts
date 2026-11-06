import type { DeviceStart } from '../devices/installation'
import type { MoveLock } from '../targets/move-question'
import type { DeviceOption, MenuDevice } from '../targets/types'

/**
 * The conversation's offline primary device, as its card above the composer
 * shows it: its name, since when it is offline, how to start its runner again,
 * and what Move to Another Device… lists.
 */
export interface OfflineDevice {
  name: string
  /** When its runner was last connected, an ISO timestamp; none for a device whose runner never connected. */
  seen?: string
  start: DeviceStart
  /** The offline device, as Run On checks it. */
  primaryDevice: MenuDevice
  /** The user's paired devices; the card lists the online ones. */
  devices: DeviceOption[]
  /** In a project: choosing a device opens its directory picker. */
  chooseDirectory?: boolean
  /** Why the conversation cannot move now: archived, or a move under way. */
  locked?: MoveLock
}
