import type { DeviceDto, DeviceKind } from '../api/generated/web-api'
import { shownMark, type DeviceMark } from '@demicodes/web-ui/devices/mark'
import type { HostKind } from '@demicodes/web-ui/hosts/types'
import { t as shared } from '@demicodes/web-ui/locales/text'
import { t } from './locales/text'

/**
 * Whether a device is the Cloud, the one managed device of an account, or a
 * paired one. The Cloud and a paired device are the same thing, a Host behind
 * a runner; this is the one place that tells them apart, for what the design
 * says differs (the name, the glyph, waking a stopped one).
 */
export function hostKindOf(device: { kind: DeviceKind } | null | undefined): HostKind {
  return device?.kind === 'managed' ? 'cloud' : 'device'
}

/** The account's Cloud among its devices, once its first use made it. */
export function cloudDeviceIn<T extends { kind: DeviceKind }>(devices: readonly T[] | undefined): T | undefined {
  return devices?.find((device) => hostKindOf(device) === 'cloud')
}

/**
 * The mark a page draws for `device`, one of the snapshot's `devices` (`conversations.md`
 * § Conversations and projects): its number only while another has its logo; null for the
 * Cloud and a device the snapshot does not list.
 */
export function deviceMarkOf(
  device: Pick<DeviceDto, 'mark'> | null | undefined,
  devices: readonly Pick<DeviceDto, 'mark'>[],
): DeviceMark | null {
  return device?.mark ? shownMark(device.mark, devices.map((other) => other.mark)) : null
}

/** What a Host is called: the Cloud, a paired device by its name, or a device the account no longer lists. */
export function hostLabel(kind: HostKind, device: { name: string } | null | undefined): string {
  return kind === 'cloud' ? shared('common.cloud') : (device?.name ?? t('hosts.unavailableDevice'))
}
