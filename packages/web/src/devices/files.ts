import {
  type FileBrowserHost,
  type FileBrowserPlaceGroup,
  type FileBrowserSource,
} from '@demicodes/web-ui/files/types'
import { fileSource } from '../api/files'
import { deviceWatch } from '../conversation/file-watch'
import type { Device, Project } from '../state/types'
import { t } from './locales/text'

/**
 * A device's folders for its folder dialog, through device access: the
 * dialog shows sizes and times, so each listing it keeps follows them on
 * the device's watch (`packages/web-ui/src/files/file-cache.ts`).
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
    groups.push({ label: t('places.quickAccess'), places: [{ path: device.home, label: t('places.home') }] })
  }
  if (places.length) {
    groups.push({ label: t('places.workspaces'), places })
  }
  return groups
}

export function browserHosts(devices: Device[]): FileBrowserHost[] {
  return devices.map((device) => ({
    id: device.id,
    label: device.name,
    state: device.state,
    mark: device.mark,
  }))
}
