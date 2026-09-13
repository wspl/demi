import { FileBrowserError, type FileBrowserSource } from '@demicodes/web-ui/files/types'
import { ApiError, apiRequest, jsonBody, readResponse } from './client'
import { directorySchema, fileTextSchema } from './contracts'
import type { Device } from '../state/types'

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
export function fileSource(
  endpoints: { directory: string | null; text?: string },
  device: Pick<Device, 'platform' | 'home'> | null,
): FileBrowserSource {
  const { directory: endpoint, text: textEndpoint } = endpoints
  const key = JSON.stringify([endpoint, textEndpoint, device?.platform, device?.home])
  const cached = sources.get(key)
  if (cached) {
    return cached
  }
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
    ...(textEndpoint
      ? {
          async read(path: string, signal?: AbortSignal) {
            try {
              const response = await apiRequest(
                `${textEndpoint}?${new URLSearchParams({ path })}`,
                { signal },
              )
              return (await readResponse(response, fileTextSchema)).text
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

