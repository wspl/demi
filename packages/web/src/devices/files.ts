import {
  type FileBrowserHost,
  type FileBrowserPlaceGroup,
  type FileBrowserSource,
} from '@demicodes/web-ui/files/types'
import { fileSource } from '../api/files'
import type { Device, Project } from '../state/types'

export function fileSourceFor(device: Device | null): FileBrowserSource {
  return fileSource(device ? `/devices/${encodeURIComponent(device.id)}/fs` : null, device)
}

export function placesFor(
  device: Device | null,
  projects: Project[],
): FileBrowserPlaceGroup[] {
  const places = projects
    .filter((project) => project.deviceId === device?.id)
    .map((project) => ({
      path: project.path,
      label: project.name,
    }))
  return [
    ...(device?.home
      ? [
          {
            label: 'Quick access',
            places: [
              {
                path: device.home,
                label: 'Home',
              },
            ],
          },
        ]
      : []),
    ...(places.length
      ? [
          {
            label: 'Workspaces',
            places,
          },
        ]
      : []),
  ]
}

export function browserHosts(devices: Device[]): FileBrowserHost[] {
  return devices.map((device) => ({
    id: device.id,
    label: device.name,
    online: device.online,
  }))
}
