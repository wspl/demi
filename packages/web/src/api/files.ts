import { HostFiles } from '@demicodes/web-ui/files/file-cache'
import { keptSource, type ContentReads, type FileFollower, type FileReads } from '@demicodes/web-ui/files/kept-source'
import {
  FileBrowserError,
  type FileBrowserFailure,
  type FileBrowserSource,
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
export function rawFileContents(endpoint: string): ContentReads {
  return {
    url: (path, options = {}) => apiUrl(`${endpoint}?${new URLSearchParams({
      path,
      ...(options.version ? { version: options.version } : {}),
      ...(options.download ? { download: 'true' } : {}),
    })}`),
    async describe(path) {
      let response: Response
      try {
        response = await apiRequest(`${endpoint}?${new URLSearchParams({ path })}`, { method: 'HEAD' })
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

/** What the page keeps of each Host's files, by its device: every conversation on a Host shares it. */
const hosts = new Map<string, HostFiles>()

/** The kept files of the Host of `deviceId`, made on first use for the page's lifetime. */
export function hostFiles(deviceId: string): HostFiles {
  let files = hosts.get(deviceId)
  if (!files) {
    files = new HostFiles()
    hosts.set(deviceId, files)
  }
  return files
}

/** The reads and writes a file source makes through `endpoints`, each a request. */
export function fileReads(
  endpoints: FileRoutes,
  device: Pick<Device, 'platform' | 'home'> | null,
): FileReads {
  const {
    directory: endpoint,
    text: textEndpoint,
    raw: rawEndpoint,
    upload: uploadEndpoint,
    remove: removeEndpoint,
  } = endpoints
  return {
    platform: device?.platform ?? 'linux',
    home: device?.home ?? '/',
    async list(path) {
      if (!endpoint) {
        throw new FileBrowserError('offline', 'Select a connected device.')
      }
      try {
        const response = await apiRequest(`${endpoint}?${new URLSearchParams({ path })}`)
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
          // The text the page holds is read again with its version: an
          // unchanged file answers 304 without it.
          async readText(path: string, held: string | null) {
            try {
              const response = await apiRequest(`${textEndpoint}?${new URLSearchParams({ path })}`, {
                headers: held === null ? {} : { 'if-none-match': held },
                allowNotModified: true,
              })
              if (response.status === 304) {
                return null
              }
              const { text } = await readResponse(response, fileTextSchema)
              return { text, version: response.headers.get('etag') }
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
}

const sources = new Map<string, FileBrowserSource>()

/**
 * The shared file browser handles paths and selection; this adapter handles
 * HTTP, over the kept files of the device's Host. A conversation's source
 * names its `follower`, the conversation's file watch.
 */
export function fileSource(
  endpoints: FileRoutes,
  device: Pick<Device, 'id' | 'platform' | 'home'> | null,
  follower?: FileFollower,
  /** Stands other reads in for the requests, as a conversation's direct channel does. */
  through: (reads: FileReads) => FileReads = (reads) => reads,
): FileBrowserSource {
  const key = JSON.stringify([endpoints, device?.id, device?.platform, device?.home])
  const cached = sources.get(key)
  if (cached) {
    return cached
  }
  // Without a device nothing is listed, so nothing is kept.
  const files = hostFiles(device?.id ?? '')
  const source = keptSource(through(fileReads(endpoints, device)), { files, follower })
  sources.set(key, source)
  return source
}
