import type { UploadFile } from '@demicodes/web-ui/agent/message-input/attachments'
import type { BlobUrl } from '@demicodes/web-ui/agent/media-source'
import { ApiError, apiError, apiRequest, apiUrl, invalidResponse } from './client'
import { attachmentAnswerSchema } from './generated/web-api'

/**
 * Where the page loads a blob of the user's own, seen as `mediaType`
 * (`web-api.md` § Uploads and media): a message's upload or a tool's medium.
 */
export const blobUrl: BlobUrl = (ref, mediaType) =>
  apiUrl(`/blobs/${encodeURIComponent(ref)}?${new URLSearchParams({ type: mediaType })}`)

/** The media type a file's bytes are sent as. */
function sentMediaType(file: File): string {
  return file.type || 'application/octet-stream'
}

/**
 * How long an upload may go without a byte moving before it fails, the
 * backend's own stall rule (`sessions-and-targets.md` § Host operations).
 */
const UPLOAD_STALL_MS = 60_000

/**
 * Sends a file's raw bytes, reporting how many have gone as they go. An
 * upload fails once nothing has moved for a minute, however long it takes
 * in all; the wait for the answer after the last byte counts too. Every
 * exit releases the XHR listeners and the timer. Resolves with the answer's
 * JSON, or null for an answer without a body.
 */
export function uploadBytes(
  path: string,
  file: File,
  options: {
    method?: 'POST' | 'PUT'
    mediaType?: string
    signal: AbortSignal
    progress: (sent: number) => void
  },
): Promise<unknown> {
  const { signal } = options
  return new Promise((resolve, reject) => {
    signal.throwIfAborted()
    const xhr = new XMLHttpRequest()
    let stall: ReturnType<typeof setTimeout> | undefined
    const cleanup = () => {
      xhr.onload = null
      xhr.onerror = null
      xhr.onabort = null
      xhr.upload.onprogress = null
      clearTimeout(stall)
      signal.removeEventListener('abort', abort)
    }
    const fail = (error: Error) => {
      cleanup()
      reject(error)
    }
    const stop = (reason: unknown) => {
      cleanup()
      xhr.abort()
      reject(reason)
    }
    const abort = () => stop(signal.reason)
    const moved = () => {
      clearTimeout(stall)
      stall = setTimeout(() => stop(new Error('Upload stalled.')), UPLOAD_STALL_MS)
    }
    xhr.open(options.method ?? 'POST', apiUrl(path))
    xhr.withCredentials = true
    xhr.setRequestHeader('Content-Type', options.mediaType ?? sentMediaType(file))
    xhr.upload.onprogress = (event) => {
      moved()
      options.progress(event.loaded)
    }
    xhr.onerror = () => fail(new Error('Upload connection failed.'))
    xhr.onabort = () => fail(new Error('Upload cancelled.'))
    xhr.onload = () => {
      if (xhr.status < 200 || xhr.status >= 300) {
        fail(apiError(xhr.status, xhr.responseText))
        return
      }
      try {
        const body: unknown = xhr.responseText ? JSON.parse(xhr.responseText) : null
        cleanup()
        resolve(body)
      } catch (error) {
        fail(error instanceof Error ? error : new Error(String(error)))
      }
    }
    signal.addEventListener('abort', abort, { once: true })
    moved()
    try {
      xhr.send(file)
    } catch (error) {
      fail(error instanceof Error ? error : new Error(String(error)))
    }
  })
}

/** The SHA-256 of a file's bytes in lowercase hexadecimal, the name of its blob. */
async function fileSha256(file: File): Promise<string> {
  const digest = await crypto.subtle.digest('SHA-256', await file.arrayBuffer())
  return new Uint8Array(digest).toHex()
}

/**
 * Asks for an upload of the blob the user holds already, which sends no
 * bytes; null when the user's blobs do not hold it and the bytes must go.
 */
async function uploadHeld(query: URLSearchParams, file: File, signal: AbortSignal): Promise<unknown> {
  try {
    const response = await apiRequest(`/attachments?${query}`, {
      method: 'POST',
      headers: { 'Content-Type': sentMediaType(file) },
      signal,
    })
    return await response.json()
  } catch (error) {
    if (error instanceof ApiError && error.code === 'blob_missing') {
      return null
    }
    throw error
  }
}

/**
 * Uploads a file a message will carry (`web-api.md` § Uploads and media):
 * its bytes become a blob of the user's, and the answer names the upload a
 * frame refers to, with the media type and the opening the backend read.
 * A file whose blob the user holds already sends no bytes: the page names
 * the file's SHA-256 first, and sends the bytes, naming it again, only when
 * the backend does not hold them. The main composer and the edit composer
 * both upload through this.
 */
export const uploadAttachment: UploadFile = async (file, options) => {
  const sha256 = await fileSha256(file)
  const query = new URLSearchParams({ name: file.name, sha256 })
  const held = await uploadHeld(query, file, options.signal)
  const answer = held ?? await uploadBytes(`/attachments?${query}`, file, {
    signal: options.signal,
    progress: (sent) => options.progress(file.size ? sent / file.size : 1),
  })
  const parsed = attachmentAnswerSchema.safeParse(answer)
  if (!parsed.success) {
    throw invalidResponse(parsed.error)
  }
  const { attachment } = parsed.data
  return {
    id: attachment.id,
    mediaType: attachment.mediaType,
    sha256: attachment.sha256,
    ...(attachment.snippet ? { snippet: attachment.snippet } : {}),
  }
}
