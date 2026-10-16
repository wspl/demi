import type { DevicePath } from '../devices/direct'
import type { DeviceMark } from '../devices/mark'
import type { DeviceState } from '../devices/state'

export interface HostDeviceOption {
  id: string
  name: string
  state: DeviceState
  /** How much of its next runner it has downloaded, while that download moves (`DeviceStatus`). */
  downloadPercent?: number | null
  /** Its mark, with the number it shows (`shownMark`); null draws the plain computer. */
  mark: DeviceMark | null
  /** The path this page found to the device; none before it connected this session. */
  path?: DevicePath | null
  /** The path this browser found last time, shown fainter while `path` is none; none for a device it never reached. */
  lastPath?: DevicePath | null
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
  /** How much of its next runner a device has downloaded, while that download moves (`DeviceStatus`). */
  downloadPercent?: number | null
  /** A device's mark, with the number it shows (`shownMark`); null for the Cloud and a device the account no longer lists. */
  mark: DeviceMark | null
}
