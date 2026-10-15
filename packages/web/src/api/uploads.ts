import type { ClaimFile, UploadedFile, UploadFile } from '@demicodes/web-ui/agent/message-input/attachments'
import type { BlobUrl } from '@demicodes/web-ui/agent/media-source'
import { coded } from '@demicodes/web-ui/infra/errors'
import { apiError, apiRequest, apiUrl, invalidResponse, readResponse, untilReached } from './client'
import { type AttachmentDto, attachmentAnswerSchema, attachmentCheckAnswerSchema } from './generated/web-api'

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
 * Sends a file's raw bytes, reporting how many have gone as they go. A try
 * that cannot reach the backend waits for it and sends the bytes again from
 * the start, as every request of the page does (`untilReached`); only the
 * backend's own answer fails the upload. Resolves with the answer's JSON,
 * or null for an answer without a body.
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
  return untilReached(() => sendBytes(path, file, options), options.signal)
}

/**
 * One try of an upload. It ends once nothing has moved for a minute, however
 * long it takes in all; the wait for the answer after the last byte counts
 * too. Every exit releases the XHR listeners and the timer.
 */
function sendBytes(
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
      // A deadline, as a request's own timeout is: the backend did not answer in time.
      stall = setTimeout(() => stop(new DOMException('Upload stalled.', 'TimeoutError')), UPLOAD_STALL_MS)
    }
    xhr.open(options.method ?? 'POST', apiUrl(path))
    xhr.withCredentials = true
    xhr.setRequestHeader('Content-Type', options.mediaType ?? sentMediaType(file))
    xhr.upload.onprogress = (event) => {
      moved()
      options.progress(event.loaded)
    }
    // No answer came, which fetch reports as a TypeError: the backend could not be reached.
    xhr.onerror = () => fail(coded(new TypeError('Upload connection failed.'), 'upload_connection_failed'))
    xhr.onabort = () => fail(coded(new Error('Upload cancelled.'), 'upload_cancelled'))
    xhr.onload = () => {
      if (xhr.status < 200 || xhr.status >= 300) {
        fail(apiError(xhr.status, xhr.responseText, undefined, xhr.statusText))
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
async function uploadHeld(query: URLSearchParams, mediaType: string, signal: AbortSignal): Promise<UploadedFile | null> {
  const response = await apiRequest(`/attachments?${query}`, {
    method: 'POST',
    headers: { 'Content-Type': mediaType },
    signal,
  })
  const { attachment } = await readResponse(response, attachmentCheckAnswerSchema)
  return attachment && uploadedFile(attachment)
}

/** The upload `attachment` names, with what the backend read of the file. */
function uploadedFile(attachment: AttachmentDto): UploadedFile {
  return {
    id: attachment.id,
    mediaType: attachment.mediaType,
    sha256: attachment.sha256,
    ...(attachment.snippet ? { snippet: attachment.snippet } : {}),
  }
}

/**
 * Makes an upload of a file a pasted message names by its blob, which sends
 * no bytes (`web-api.md` § Uploads and media); null when the user's blobs do
 * not hold it, as one of another account or one since deleted.
 */
export const claimStoredFile: ClaimFile = async (file, signal) => {
  const query = new URLSearchParams({ name: file.name, sha256: file.sha256 })
  return uploadHeld(query, file.mediaType, signal)
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
  const held = await uploadHeld(query, sentMediaType(file), options.signal)
  if (held) {
    return held
  }
  const sent = attachmentAnswerSchema.safeParse(await uploadBytes(`/attachments?${query}`, file, {
    signal: options.signal,
    progress: (sent) => options.progress(file.size ? sent / file.size : 1),
  }))
  if (!sent.success) {
    throw invalidResponse(sent.error)
  }
  return uploadedFile(sent.data.attachment)
}
