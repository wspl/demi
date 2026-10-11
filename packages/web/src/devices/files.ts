import {
  type FileBrowserHost,
  type FileBrowserPlaceGroup,
  type FileBrowserSource,
} from '@demicodes/web-ui/files/types'
import { fileSource } from '../api/files'
import { deviceWatch } from '../conversation/file-watch'
import type { Device, Project } from '../state/types'

/**
 * A device's folders for its folder dialog, through device access: the
 * dialog shows sizes and times, so each listing it keeps follows them on
 * the device's watch (`plugin-pages.md` § What the service keeps).
 */
export function fileSourceFor(device: Device | null): FileBrowserSource {
  return fileSource({
    directory: device ? `/devices/${encodeURIComponent(device.id)}/fs` : null,
  }, device, {
    details: true,
    watch: (files) => device ? deviceWatch(device.id, files) : null,
  })
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
  const groups: FileBrowserPlaceGroup[] = []
  if (device?.home) {
    groups.push({ label: 'Quick Access', places: [{ path: device.home, label: 'Home' }] })
  }
  if (places.length) {
    groups.push({ label: 'Workspaces', places })
  }
  return groups
}

export function browserHosts(devices: Device[]): FileBrowserHost[] {
  return devices.map((device) => ({
    id: device.id,
    label: device.name,
    state: device.state,
  }))
}
