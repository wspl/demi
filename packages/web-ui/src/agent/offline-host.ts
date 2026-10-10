import type { DeviceStart } from '../devices/installation'
import type { HostDeviceOption, HostMenuHost } from '../hosts/types'

/**
 * The conversation's offline primary Host, as its card above the composer
 * shows it (`product.md` § Recovering an unfinished turn): its name, how to
 * start its runner again, and what Move to Another Host… lists.
 */
export interface OfflineHost {
  name: string
  start: DeviceStart
  /** The offline device, as Run On checks it. */
  primaryHost: HostMenuHost
  /** The user's paired devices; the card lists the online ones. */
  devices: HostDeviceOption[]
  /** In a project: choosing a device opens its directory picker. */
  chooseDirectory?: boolean
  /** The conversation cannot move now, as while its work runs. */
  locked?: boolean
}
