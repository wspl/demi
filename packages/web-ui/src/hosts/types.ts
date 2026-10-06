import type { DeviceState } from '../devices/state'

export interface HostDeviceOption {
  id: string
  name: string
  state: DeviceState
}

export interface HostMenuHost extends HostDeviceOption {
  kind: 'cloud' | 'device'
}
