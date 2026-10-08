import type { DevicePath } from '../devices/direct'
import type { DeviceState } from '../devices/state'

export interface HostDeviceOption {
  id: string
  name: string
  state: DeviceState
  /** The path this page last found to the device; none before it connected this session. */
  path?: DevicePath | null
}

export interface HostMenuHost {
  id: string
  name: string
  kind: 'cloud' | 'device'
  state: DeviceState
}
