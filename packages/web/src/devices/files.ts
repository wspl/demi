import {
  FileBrowserError,
  type FileBrowserHost,
  type FileBrowserPlaceGroup,
  type FileBrowserSource,
} from '@demicodes/web-ui/files/types'
import { ApiError, apiRequest, jsonBody, readResponse } from '../api/client'
import { directorySchema } from '../api/contracts'
import type { Device, Project } from '../state/types'

function browserError(error: unknown): never {
  if (error instanceof ApiError) {
    const kind =
      error.code === 'device_offline'
        ? 'offline'
        : error.status === 404
          ? 'not-found'
          : error.status === 403
            ? 'permission'
            : 'other'
    throw new FileBrowserError(kind, error.message)
  }
  throw error
}

const sources = new Map<string, FileBrowserSource>()

/** The shared browser handles paths and selection; this adapter handles HTTP. */
export function fileSourceFor(device: Device | null): FileBrowserSource {
  const key = JSON.stringify([device?.id, device?.platform, device?.home])
  const cached = sources.get(key)
  if (cached) {
    return cached
  }
  const endpoint = device ? `/devices/${encodeURIComponent(device.id)}/fs` : null
  const source: FileBrowserSource = {
    platform: device?.platform ?? 'linux',
    home: device?.home ?? '/',
    async list(path, signal) {
      if (!endpoint) {
        throw new FileBrowserError('offline', 'Select a connected device.')
      }
      try {
        const response = await apiRequest(
          `${endpoint}?${new URLSearchParams({ path })}`,
          { signal },
        )
        const directory = await readResponse(response, directorySchema)
        return directory.entries.map((entry) => ({
          name: entry.name,
          isDirectory: entry.isDirectory,
          size: entry.size ?? undefined,
          modifiedAt: entry.modifiedAt ?? undefined,
        }))
      } catch (error) {
        browserError(error)
      }
    },
    ...(endpoint
      ? {
          async createDirectory(path: string, signal?: AbortSignal) {
            try {
              await apiRequest(endpoint, {
                method: 'POST',
                signal,
                ...jsonBody({ path }),
              })
            } catch (error) {
              browserError(error)
            }
          },
        }
      : {}),
  }
  sources.set(key, source)
  return source
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
