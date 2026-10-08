import type { DeviceState } from '../devices/state'

/**
 * How this page reaches a device (`direct-channel.md` § What the user sees):
 * the device it runs on, over the local network, a P2P connection, or the
 * server's relay.
 */
export type HostPath = 'this-computer' | 'lan' | 'p2p' | 'relay'

/** How a path reads at the end of a device's row in the host menu. */
export const HOST_PATH_LABEL: Record<HostPath, string> = {
  'this-computer': 'This Computer',
  lan: 'LAN',
  p2p: 'P2P',
  relay: 'Relay',
}

export interface HostDeviceOption {
  id: string
  name: string
  state: DeviceState
  /** The path this page last found to the device; none before it connected this session. */
  path?: HostPath
}

export interface HostMenuHost {
  id: string
  name: string
  kind: 'cloud' | 'device'
  state: DeviceState
}
