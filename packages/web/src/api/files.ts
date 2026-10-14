import { HostFiles } from '@demicodes/web-ui/files/file-cache'
import { keptSource, type ContentReads, type FileReads } from '@demicodes/web-ui/files/kept-source'
import type { ReadAt, WatchChannel } from '@demicodes/web-ui/files/watch-channel'
import {
  FileBrowserError,
  type FileBrowserEntry,
  type FileBrowserFailure,
  type FileBrowserLink,
  type FileBrowserSource,
  type FileUploadOptions,
} from '@demicodes/web-ui/files/types'
import { z } from 'zod'
import { ApiError, apiRequest, apiUrl, invalidResponse, jsonBody, readResponse } from './client'
import { directorySchema, fileTextSchema, type CreateDirectory, type DirectoryEntry, type LinkTarget } from './generated/web-api'
import { uploadBytes } from './uploads'
import type { Device } from '../state/types'
import type { SentenceText } from '@demicodes/web-ui/ui/ui-text'

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

/**
 * `X-Demi-Read-At: <epoch>.<number>`, the Host's number before it read
 * (`web-api.md` § File text and working tree changes).
 */
const readAtSchema = z.string().regex(/^.+\.\d+$/).transform((value): ReadAt => {
  const dot = value.lastIndexOf('.')
  return { epoch: value.slice(0, dot), number: Number(value.slice(dot + 1)) }
})

/** The Host's number an answer carries; null for one that names none, which is never current. */
export function readAtOf(headers: Headers): ReadAt | null {
  const value = headers.get('x-demi-read-at')
  if (value === null)
    return null
  const parsed = readAtSchema.safeParse(value)
  if (!parsed.success)
    throw invalidResponse(parsed.error)
  return parsed.data
}

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

/**
 * What the file views say of a failure, after their own line such as
 * “Could not read this file.” (the gallery's Writing page): the page's
 * sentence for what happened, never the backend's message, which names its
 * internals, such as a Cloud runner's reconnect timeout.
 */
function failureSentence(error: ApiError, kind: FileBrowserFailure['kind']): SentenceText {
  switch (error.code) {
    case 'host_stopped':
      return 'The Cloud is stopped.'
    case 'cloud_resetting':
      return 'The Cloud is resetting.'
    case 'cloud_capacity':
      return 'The Cloud cannot start now: the server has no room for it.'
    case 'cloud_crash_loop':
      return 'The Cloud stopped starting after repeated failures.'
    case 'cloud_unavailable':
      return 'The Cloud is not available right now.'
    case 'conversation_archived':
      return 'This conversation is archived.'
    case 'conversation_busy':
      return 'The conversation is changing its Host.'
  }
  switch (kind) {
    case 'offline':
      return 'The device is offline.'
    case 'not-found':
      return 'It is no longer there.'
    case 'permission':
      return 'The Host does not allow access to it.'
    case 'exists':
      return 'Something of that name is there already.'
    case 'binary':
      return 'It is not text.'
    case 'too-large':
      return 'It is too large.'
    case 'other':
      return 'The Host did not answer.'
  }
}

/**
 * An API failure as the file views read it, in the page's words, with the
 * file's version and the Host's number a refusal that is the Host's answer
 * carries; anything else as it is.
 */
export function fileBrowserError(error: unknown): unknown {
  if (!(error instanceof ApiError))
    return error
  const kind = failureKind(error)
  return new FileBrowserError(kind, failureSentence(error, kind), {
    version: error.headers.get('etag'),
    readAt: readAtOf(error.headers),
    requestId: error.requestId,
  })
}

function browserError(error: unknown): never {
  throw fileBrowserError(error)
}

/**
 * The bytes behind a raw file route (`web-api.md` § File text and working
 * tree changes): `endpoint` serves `?path=`, with `version` and `download`,
 * and answers `HEAD` with a file's size, modification time and version.
 */
export function rawFileContents(endpoint: string, fixed: Record<string, string> = {}): ContentReads {
  return {
    url: (path, options = {}) => apiUrl(`${endpoint}?${new URLSearchParams({
      ...fixed,
      path,
      ...(options.version ? { version: options.version } : {}),
      ...(options.download ? { download: 'true' } : {}),
    })}`),
    async describe(path) {
      let response: Response
      try {
        response = await apiRequest(`${endpoint}?${new URLSearchParams({ ...fixed, path })}`, { method: 'HEAD' })
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
      return { value: headers.data, readAt: readAtOf(response.headers) }
    },
    // One byte range, which the route answers with exactly those bytes.
    async readStart(path, length, options = {}) {
      const query = new URLSearchParams({ ...fixed, path, ...(options.version ? { version: options.version } : {}) })
      let response: Response
      try {
        response = await apiRequest(`${endpoint}?${query}`, {
          headers: { range: `bytes=0-${length - 1}` },
          signal: options.signal,
        })
      } catch (error) {
        browserError(error)
      }
      return new Uint8Array(await response.arrayBuffer()).subarray(0, length)
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

/**
 * A listing's entry as the file browser lists it, over the relay or the
 * direct channel alike: a symbolic link is what its target is, a folder
 * when it leads to a directory, and keeps where it points
 * (`file-previews.md` § Symbolic links).
 */
export function fileBrowserEntry(entry: DirectoryEntry): FileBrowserEntry {
  const { link } = entry
  return {
    name: entry.name,
    isDirectory: entry.isDirectory || link?.leadsTo.kind === 'directory',
    size: entry.size,
    modifiedAt: entry.modifiedAt,
    ...(link ? { link: { target: link.target, ...linkBreak(link.leadsTo) } } : {}),
  }
}

/** Why a link's target cannot be reached, as the listing says; nothing while it can. */
function linkBreak(leadsTo: LinkTarget): Pick<FileBrowserLink, 'broken'> {
  switch (leadsTo.kind) {
    case 'missing':
      return { broken: { kind: 'missing' } }
    case 'unreadable':
      return { broken: { kind: 'unreadable', reason: leadsTo.reason } }
    default:
      return {}
  }
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
        return { value: directory.entries.map(fileBrowserEntry), readAt: readAtOf(response.headers) }
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
          async readText(path: string, held: string | null): ReturnType<NonNullable<FileReads['readText']>> {
            try {
              const response = await apiRequest(`${textEndpoint}?${new URLSearchParams({ path })}`, {
                headers: held === null ? {} : { 'if-none-match': held },
                allowNotModified: true,
              })
              const readAt = readAtOf(response.headers)
              if (response.status === 304) {
                return { unchanged: true, readAt }
              }
              const { text } = await readResponse(response, fileTextSchema)
              return { value: { text, version: response.headers.get('etag') }, readAt }
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

/** How a file source keeps what it reads current. */
export interface FileSourceOptions {
  /**
   * The watch of the source's route, which carries the subscriptions of
   * what it keeps: a conversation's, or a device's for its folder dialog.
   * A route has one watch, so a source is made once per route.
   */
  watch?: (files: HostFiles) => WatchChannel | null
  /** Its views show sizes and times, as the folder dialogs do, which its listings then follow. */
  details?: boolean
  /** Stands other reads in for the requests, as a conversation's direct channel does. */
  through?: (reads: FileReads) => FileReads
}

/**
 * The shared file browser handles paths and selection; this adapter handles
 * HTTP, over the kept files of the device's Host, which the route's watch
 * keeps current.
 */
export function fileSource(
  endpoints: FileRoutes,
  device: Pick<Device, 'id' | 'platform' | 'home'> | null,
  { watch, details = false, through = (reads) => reads }: FileSourceOptions = {},
): FileBrowserSource {
  const key = JSON.stringify([endpoints, device?.id, device?.platform, device?.home, details])
  const cached = sources.get(key)
  if (cached) {
    return cached
  }
  // Without a device nothing is listed, so nothing is kept.
  const files = hostFiles(device?.id ?? '')
  const source = keptSource(through(fileReads(endpoints, device)), {
    files,
    watch: device ? watch?.(files) ?? null : null,
    details,
  })
  sources.set(key, source)
  return source
}
