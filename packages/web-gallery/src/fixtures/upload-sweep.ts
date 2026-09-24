import type { UserContentBlock } from '@demicodes/protocol'
import { delay } from '@demicodes/utils'
import type { MessageEditContent } from '@demicodes/web-ui/agent/message-editing'
import { clampUnit, type UploadedFile, type UploadFile } from '@demicodes/web-ui/agent/message-input/attachments'

const PROTOTYPE_UPLOAD_MS = 1400
/** How much of a text file the backend's answer carries (`web-api.md` § Uploads and media). */
const SNIPPET_CHARS = 160

/** Prototype host: an upload that sweeps from 0 to 1 in PROTOTYPE_UPLOAD_MS, a report per frame. */
export async function sweepUpload(signal: AbortSignal, report: (progress: number) => void): Promise<void> {
  const started = performance.now()
  while (!signal.aborted) {
    const progress = clampUnit((performance.now() - started) / PROTOTYPE_UPLOAD_MS)
    report(progress)
    if (progress >= 1)
      return
    await delay(16, signal)
  }
}

/**
 * The gallery's stand-in for the backend's uploads: a file sweeps its
 * progress as a slow network would, and the answer says what the backend
 * would, a text file's opening among it. The files stay in the page under
 * their blob names, so a message that received one can show its picture.
 */
export function galleryUploads() {
  const blobs = new Map<string, { file: File; url: string }>()

  const upload: UploadFile = async (file, options) => {
    await sweepUpload(options.signal, options.progress)
    options.signal.throwIfAborted()
    const sha256 = [...crypto.getRandomValues(new Uint8Array(32))].map((byte) => byte.toString(16).padStart(2, '0')).join('')
    blobs.set(sha256, { file, url: URL.createObjectURL(file) })
    const answer: UploadedFile = { id: crypto.randomUUID(), mediaType: file.type || 'application/octet-stream', sha256 }
    if (file.type.startsWith('text/')) {
      const text = await file.slice(0, SNIPPET_CHARS * 4).text()
      answer.snippet = text.replace(/\r\n?/g, '\n').trimStart().slice(0, SNIPPET_CHARS)
    }
    return answer
  }

  /**
   * An edit's content as the transcript would record it once the backend
   * took it: each file the edit added becomes its attachment record, with its
   * picture before it when it is one.
   */
  function received(content: readonly MessageEditContent[]): UserContentBlock[] {
    return content.flatMap((part): UserContentBlock[] => {
      if (part.type !== 'upload') {
        return [part]
      }
      const blob = blobs.get(part.sha256)
      const record: UserContentBlock = {
        type: 'attachment',
        name: part.fileName,
        path: `/home/demi/.demi/attachments/gallery/${part.fileName}`,
        mediaType: part.mediaType,
        sizeBytes: blob?.file.size ?? 0,
        sha256: part.sha256,
        ...(part.snippet ? { snippet: part.snippet } : {}),
      }
      return part.mediaType.startsWith('image/') && blob
        ? [{ type: 'image', source: { type: 'url', url: blob.url } }, record]
        : [record]
    })
  }

  /** Lets the page's copies go, when the specimen that holds them does. */
  function release(): void {
    for (const blob of blobs.values()) {
      URL.revokeObjectURL(blob.url)
    }
    blobs.clear()
  }

  return { upload, received, release }
}
