import type { DeviceKind } from '../api/generated/web-api'
import type { HostKind } from '@demicodes/web-ui/hosts/types'

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

/** What a Host is called: the Cloud, a paired device by its name, or a device the account no longer lists. */
export function hostLabel(kind: HostKind, device: { name: string } | null | undefined): string {
  return kind === 'cloud' ? 'Cloud' : (device?.name ?? 'Unavailable device')
}
