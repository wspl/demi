import {
  FileBrowserError,
  type FileBrowserFailure,
  type FileBrowserSource,
  type FileContents,
  type FileUploadOptions,
} from '@demicodes/web-ui/files/types'
import { z } from 'zod'
import { ApiError, apiRequest, apiUrl, invalidResponse, jsonBody, readResponse } from './client'
import { directorySchema, fileTextSchema, type CreateDirectory } from './generated/web-api'
import { uploadBytes } from './uploads'
import type { Device } from '../state/types'

/**
 * A raw file answer's headers as a file description (`web-api.md` § File
 * text and working tree changes): its size, its modification time when the
 * route knows it, and the version a preview pins.
 */
const fileHeadersSchema = z.object({
  size: z.string().regex(/^\d+$/).transform(Number),
  modifiedAt: z.string().nullable().transform((value, context) => {
    if (value === null)
      return null
    const time = Date.parse(value)
    if (Number.isNaN(time)) {
      context.addIssue('Expected an HTTP date')
      return z.NEVER
    }
    return new Date(time).toISOString()
  }),
  version: z.string().nullable(),
})

/** What an API failure means to the file views; a `HEAD` answer has only its status to say it. */
function failureKind(error: ApiError): FileBrowserFailure['kind'] {
  if (error.code === 'device_offline')
    return 'offline'
  if (error.code === 'file_exists')
    return 'exists'
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

/** The routes a file source reads and writes through; each one it lacks leaves that capability out. */
export interface FileRoutes {
  /** Lists a directory, and makes one with `POST`. */
  directory: string | null
  /** A file's text. */
  text?: string
  /** A file's bytes. */
  raw?: string
  /** Takes a file's bytes with `PUT`. */
  upload?: string
  /** Deletes a file or a directory with `DELETE`. */
  remove?: string
}

/**
 * A conversation's Host file routes (`web-api.md` § File text and working
 * tree changes): its directory listing, file text and raw bytes, uploads
 * and deletes.
 */
export function conversationFileRoutes(conversationId: string): Required<FileRoutes> {
  const base = `/conversations/${encodeURIComponent(conversationId)}/fs`
  return { directory: base, text: `${base}/file`, raw: `${base}/raw`, upload: `${base}/raw`, remove: base }
}

const sources = new Map<string, FileBrowserSource>()

/** The shared browser handles paths and selection; this adapter handles HTTP. */
export function fileSource(
  endpoints: FileRoutes,
  device: Pick<Device, 'platform' | 'home'> | null,
): FileBrowserSource {
  const {
    directory: endpoint,
    text: textEndpoint,
    raw: rawEndpoint,
    upload: uploadEndpoint,
    remove: removeEndpoint,
  } = endpoints
  const key = JSON.stringify([endpoint, textEndpoint, rawEndpoint, uploadEndpoint, removeEndpoint, device?.platform, device?.home])
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
          size: entry.size,
          modifiedAt: entry.modifiedAt,
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
                ...jsonBody({ path } satisfies CreateDirectory),
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
    ...(uploadEndpoint
      ? {
          async upload(path: string, file: File, options: FileUploadOptions) {
            const query = new URLSearchParams({ path, replace: String(options.replace) })
            try {
              await uploadBytes(`${uploadEndpoint}?${query}`, file, {
                method: 'PUT',
                signal: options.signal,
                progress: options.progress,
              })
            } catch (error) {
              browserError(error)
            }
          },
        }
      : {}),
    ...(removeEndpoint
      ? {
          async remove(path: string, signal?: AbortSignal) {
            try {
              await apiRequest(`${removeEndpoint}?${new URLSearchParams({ path })}`, { method: 'DELETE', signal })
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

