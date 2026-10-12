import type { DevicePath } from '../devices/direct'
import type { DeviceState } from '../devices/state'

export interface HostDeviceOption {
  id: string
  name: string
  state: DeviceState
  /** The path this page last found to the device; none before it connected this session. */
  path?: DevicePath | null
}

/** A Host chosen to run on: the Cloud, or a device by its id. */
export type HostChoice = { kind: 'cloud' } | { kind: 'device'; id: string }

/** A Host is the Cloud or a device. */
export type HostKind = 'cloud' | 'device'

export interface HostMenuHost {
  id: string
  name: string
  kind: HostKind
  state: DeviceState
}
