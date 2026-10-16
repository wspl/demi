import {
  mdiApple,
  mdiArch,
  mdiCentos,
  mdiDebian,
  mdiFedora,
  mdiLinux,
  mdiLinuxMint,
  mdiMicrosoftWindows,
  mdiRedhat,
  mdiUbuntu,
} from '@mdi/js'

/**
 * The logo of a device's mark (`conversations.md` § Conversations and projects): its system's, Apple
 * for macOS, the Windows logo, a Linux distribution's, or Tux for any other Linux.
 */
export type DeviceLogo =
  | 'apple'
  | 'windows'
  | 'ubuntu'
  | 'debian'
  | 'fedora'
  | 'arch'
  | 'centos'
  | 'redhat'
  | 'mint'
  | 'tux'

/**
 * A paired device's mark: its logo, and its number among the user's devices of that logo, null
 * before its runner first connected. Where it is drawn, `number` is the one it shows, which
 * `shownMark` gives.
 */
export interface DeviceMark {
  logo: DeviceLogo
  number: number | null
}

/**
 * Each logo's path, in a 24 by 24 box: Material Design Icons (Pictogrammers, Apache-2.0), from
 * `@mdi/js`.
 */
export const DEVICE_LOGO_PATH: Record<DeviceLogo, string> = {
  apple: mdiApple,
  windows: mdiMicrosoftWindows,
  ubuntu: mdiUbuntu,
  debian: mdiDebian,
  fedora: mdiFedora,
  arch: mdiArch,
  centos: mdiCentos,
  redhat: mdiRedhat,
  mint: mdiLinuxMint,
  tux: mdiLinux,
}

/**
 * The mark a page draws for `mark`, one of `marks`, the marks of all the user's devices: its
 * number shows only while another device has the same logo, so pairing a second Mac adds a 1 to
 * the first.
 */
export function shownMark(mark: DeviceMark, marks: readonly (DeviceMark | null)[]): DeviceMark {
  const alike = marks.filter((other) => other?.logo === mark.logo).length
  return { logo: mark.logo, number: alike > 1 ? mark.number : null }
}
