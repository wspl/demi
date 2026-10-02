import type { DeviceDto } from '../api/generated/web-api'

/**
 * The name the session tools show for the user's device `id`: the Cloud by
 * its product name, a device by its own, and a device the snapshot no longer
 * knows by its id, so its row stays removable.
 */
export function hostName(devices: readonly DeviceDto[], id: string): string {
  const device = devices.find((candidate) => candidate.id === id)
  if (!device) {
    return id
  }
  return device.kind === 'managed' ? 'Cloud' : device.name
}
