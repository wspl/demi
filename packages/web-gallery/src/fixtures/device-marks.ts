import { shownMark, type DeviceMark } from '@demicodes/web-ui/devices/mark'

/**
 * The marks the gallery's devices show, as the product's `shownMark` gives them for an account
 * with three Macs, a Windows PC and Linux computers of several distributions: the Macs read 1, 2
 * and 3, and the others, each alone of its logo, show no number.
 */
export const GALLERY_MARKS = {
  mac: { logo: 'apple', number: 1 },
  macMini: { logo: 'apple', number: 2 },
  macPro: { logo: 'apple', number: 3 },
  windows: { logo: 'windows', number: null },
  ubuntu: { logo: 'ubuntu', number: null },
  debian: { logo: 'debian', number: null },
  fedora: { logo: 'fedora', number: null },
  tux: { logo: 'tux', number: null },
} as const satisfies Record<string, DeviceMark>

/**
 * `devices`, the whole of a specimen's account, with the marks they show among themselves
 * (`shownMark`): a Mac alone of its logo there shows no number.
 */
export function shownAmong<T extends { mark: DeviceMark | null }>(devices: T[]): T[] {
  const marks = devices.map((device) => device.mark)
  return devices.map((device) => ({ ...device, mark: device.mark && shownMark(device.mark, marks) }))
}
