import type { ExposeMenuEntry } from '@demicodes/web-ui/hosts/types'
import type { DeviceDto } from '../api/generated/web-api'
import type { Expose } from '../api/unported'

/**
 * The snapshot's exposes as the session tools menu lists them: the host name
 * comes from the device list, the Cloud by its product name, and a device
 * the snapshot no longer knows keeps its id so the row stays removable.
 */
export function sessionToolsExposes(
  exposes: readonly Expose[],
  devices: readonly DeviceDto[],
): ExposeMenuEntry[] {
  return exposes.map((expose) => {
    const device = devices.find((candidate) => candidate.id === expose.deviceId)
    const hostName = device
      ? device.kind === 'managed' ? 'Cloud' : device.name
      : expose.deviceId
    return {
      id: expose.id,
      address: expose.address,
      hostName,
      url: expose.url,
      expiresAt: expose.expiresAt,
    }
  })
}
