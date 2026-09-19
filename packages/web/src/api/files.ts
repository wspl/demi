import {
  FileBrowserError,
  type FileBrowserFailure,
  type FileBrowserSource,
  type FileContents,
} from '@demicodes/web-ui/files/types'
import { ApiError, apiRequest, apiUrl, invalidResponse, jsonBody, readResponse } from './client'
import { directorySchema, fileHeadersSchema, fileTextSchema } from './contracts'
import type { Device } from '../state/types'

/** What an API failure means to the file views; a `HEAD` answer has only its status to say it. */
function failureKind(error: ApiError): FileBrowserFailure['kind'] {
  if (error.code === 'device_offline')
    return 'offline'
  if (error.code === 'not_text' || error.status === 415)
    return 'binary'
  if (error.code === 'file_too_large' || error.status === 413)
    return 'too-large'
  if (error.status === 404)
    return 'not-found'
  if (error.status === 403)
    return 'permission'
  return 'other'
}

/** An API failure as the file views read it; anything else as it is. */
export function fileBrowserError(error: unknown): unknown {
  return error instanceof ApiError ? new FileBrowserError(failureKind(error), error.message) : error
}

function browserError(error: unknown): never {
  throw fileBrowserError(error)
}

/**
 * The bytes behind a raw file route (`web-api.md` § File text and working
 * tree changes): `endpoint` serves `?path=`, with `version` and `download`,
 * and answers `HEAD` with a file's size, modification time and version.
 */
export function rawFileContents(endpoint: string): FileContents {
  return {
    url: (path, options = {}) => apiUrl(`${endpoint}?${new URLSearchParams({
      path,
      ...(options.version ? { version: options.version } : {}),
      ...(options.download ? { download: 'true' } : {}),
    })}`),
    async describe(path, signal) {
      let response: Response
      try {
        response = await apiRequest(`${endpoint}?${new URLSearchParams({ path })}`, { method: 'HEAD', signal })
      } catch (error) {
        browserError(error)
      }
      const headers = fileHeadersSchema.safeParse({
        size: response.headers.get('content-length'),
        modifiedAt: response.headers.get('last-modified'),
        version: response.headers.get('etag'),
      })
      if (!headers.success)
        throw invalidResponse(headers.error)
      return headers.data
    },
  }
}

/**
 * A conversation's Host file routes (`web-api.md` § File text and working
 * tree changes): its directory listing, file text and raw bytes.
 */
export function conversationFileRoutes(conversationId: string): { directory: string; text: string; raw: string } {
  const base = `/conversations/${encodeURIComponent(conversationId)}/fs`
  return { directory: base, text: `${base}/file`, raw: `${base}/raw` }
}

const sources = new Map<string, FileBrowserSource>()

/** The shared browser handles paths and selection; this adapter handles HTTP. */
export function fileSource(
  endpoints: { directory: string | null; text?: string; raw?: string },
  device: Pick<Device, 'platform' | 'home'> | null,
): FileBrowserSource {
  const { directory: endpoint, text: textEndpoint, raw: rawEndpoint } = endpoints
  const key = JSON.stringify([endpoint, textEndpoint, rawEndpoint, device?.platform, device?.home])
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
    ...(rawEndpoint ? { contents: rawFileContents(rawEndpoint) } : {}),
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

