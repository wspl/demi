import type { DeviceDto, DeviceKind } from '../api/generated/web-api'
import { shownMark, type DeviceMark } from '@demicodes/web-ui/devices/mark'
import type { TargetKind } from '@demicodes/web-ui/targets/types'
import { t as shared } from '@demicodes/web-ui/locales/text'
import { t } from './locales/text'

/**
 * Whether a device is the Cloud, the one managed device of an account, or a
 * paired one. The Cloud and a paired device are the same thing, a device behind
 * a runner; this is the one place that tells them apart, for what the design
 * says differs (the name, the glyph, waking a stopped one).
 */
export function targetKindOf(device: { kind: DeviceKind } | null | undefined): TargetKind {
  return device?.kind === 'managed' ? 'cloud' : 'device'
}

/** The account's Cloud among its devices, once its first use made it. */
export function cloudDeviceIn<T extends { kind: DeviceKind }>(devices: readonly T[] | undefined): T | undefined {
  return devices?.find((device) => targetKindOf(device) === 'cloud')
}

/**
 * The mark a page draws for `device`, one of the snapshot's `devices`: its number only
 * while another has its logo; null for the Cloud and a device the snapshot does not list.
 */
export function deviceMarkOf(
  device: Pick<DeviceDto, 'mark'> | null | undefined,
  devices: readonly Pick<DeviceDto, 'mark'>[],
): DeviceMark | null {
  return device?.mark ? shownMark(device.mark, devices.map((other) => other.mark)) : null
}

/** What a device is called: the Cloud, a paired device by its name, or a device the account no longer lists. */
export function deviceLabel(kind: TargetKind, device: { name: string } | null | undefined): string {
  return kind === 'cloud' ? shared('common.cloud') : (device?.name ?? t('devices.unavailableDevice'))
}
