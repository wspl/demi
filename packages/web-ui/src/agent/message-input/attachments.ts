import { baseName } from '@demicodes/utils'
import { formatBytes } from '../../files/format'
import { t } from '../locales/text'

/**
 * A failed upload keeps its capsule, which offers Retry; the toast says why it
 * failed. `attaching` is a file a pasted message names, which the backend
 * makes an upload of from the user's own stored file, sending no bytes.
 * `missing` is a file of a message that came back to the composer, such as
 * one not delivered, whose bytes and upload are gone: the user deletes it or
 * attaches it again.
 */
export type AttachmentPhase = 'attaching' | 'uploading' | 'ready' | 'failed' | 'missing'

/**
 * A local file on its way to the message. It is uploaded as it is added, and
 * the message names the upload; on send the backend writes the file to the
 * Host (`composer.md` § Attachments).
 */
export interface ComposerFileAttachment {
  kind: 'file'
  id: string
  name: string
  src?: string
  phase: AttachmentPhase
  /** 0–1 while `phase` is `uploading`. */
  progress?: number
  /** The opening of a text file, as the backend answered its upload. */
  snippet?: string
}

/** A host path. The tile is the same; the tooltip is `host · filename`. */
export interface ComposerRemoteAttachment {
  kind: 'reference'
  id: string
  name: string
  host: string
  path: string
}

export type ComposerAttachment = ComposerFileAttachment | ComposerRemoteAttachment

export type ComposerFileInput = Pick<ComposerFileAttachment, 'name'> &
  Partial<ComposerFileAttachment>
export type ComposerRemoteInput = Pick<ComposerRemoteAttachment, 'host' | 'path'> &
  Partial<ComposerRemoteAttachment>
export type ComposerAttachmentInput = ComposerFileInput | ComposerRemoteInput

export function isComposerFile(
  item: ComposerAttachment,
): item is ComposerFileAttachment {
  return item.kind === 'file'
}

export interface AttachmentUploadUpdate {
  phase: AttachmentPhase
  progress?: number
}

/**
 * What the backend answered for an upload (`web-api-hosts.md` § Uploads and
 * media): the id a message names the file by, the media type it read from
 * the bytes, where the bytes are in the user's blobs, and a text file's
 * opening, which the capsule shows.
 */
export interface UploadedFile {
  id: string
  mediaType: string
  sha256: string
  snippet?: string
}

/**
 * The product's upload of one file, which the main composer and the edit
 * composer share: it sends the bytes, reports the fraction sent, and answers
 * what the backend made of the file.
 */
export type UploadFile = (
  file: File,
  options: { signal: AbortSignal; progress: (fraction: number) => void },
) => Promise<UploadedFile>

/**
 * The product's upload of a file the user's blobs hold already, such as one a
 * pasted message names: no bytes go, and the answer is an upload as any
 * file's is; null when the user's blobs do not hold it.
 */
export type ClaimFile = (
  file: { name: string; sha256: string; mediaType: string },
  signal: AbortSignal,
) => Promise<UploadedFile | null>

export const ATTACHMENT_MAX_BYTES = 25 * 1024 * 1024

/**
 * A paste this long is a file, not typing: it lands in the composer as
 * `pasted-text.txt`, keeping the draft readable and the text intact.
 */
export const PASTE_AS_FILE_MIN_CHARS = 2000
export const PASTE_AS_FILE_MIN_LINES = 40
const PASTED_TEXT_NAME = 'pasted-text.txt'

export function pastedTextIsLong(text: string): boolean {
  if (text.length >= PASTE_AS_FILE_MIN_CHARS) {
    return true
  }
  let lines = 1
  for (const char of text) {
    if (char === '\n') {
      lines += 1
      if (lines >= PASTE_AS_FILE_MIN_LINES) {
        return true
      }
    }
  }
  return false
}

/**
 * The file a long paste becomes. Every one is `pasted-text.txt`, as every
 * pasted picture is `image.png`: a message holds files of one name, and the
 * Host keeps each under a suffix of its own (`composer.md` § Attachments).
 */
export function pastedTextFile(text: string): File {
  return new File([text], PASTED_TEXT_NAME, { type: 'text/plain' })
}

export function clampUnit(value: number): number {
  if (!Number.isFinite(value) || value <= 0) {
    return 0
  }
  if (value >= 1) {
    return 1
  }
  return value
}

/** How a transfer's update shows on a file: the progress of a file without bytes to send is its attaching. */
export function attachingUpdate(update: AttachmentUploadUpdate, hasBytes: boolean): AttachmentUploadUpdate {
  return update.phase === 'uploading' && !hasBytes ? { phase: 'attaching' } : update
}

export function attachmentProgress(
  item: Pick<ComposerFileAttachment, 'phase' | 'progress'>,
): number {
  if (item.phase !== 'uploading') {
    return 0
  }
  return clampUnit(item.progress ?? 0)
}

export function applyAttachmentUpdate(
  item: ComposerAttachment,
  update: AttachmentUploadUpdate,
): void {
  if (!isComposerFile(item)) {
    return
  }
  item.phase = update.phase
  item.progress =
    update.phase === 'uploading' ? clampUnit(update.progress ?? 0) : undefined
}

export function composerRemoteAttachment(
  input: ComposerRemoteInput,
): ComposerRemoteAttachment {
  return {
    kind: 'reference',
    id: input.id ?? crypto.randomUUID(),
    name: input.name ?? baseName(input.path),
    host: input.host,
    path: input.path,
  }
}

export function composerAttachment(
  input: ComposerRemoteInput,
): ComposerRemoteAttachment
export function composerAttachment(input: ComposerFileInput): ComposerFileAttachment
export function composerAttachment(
  input: ComposerAttachmentInput,
): ComposerAttachment
export function composerAttachment(
  input: ComposerAttachmentInput,
): ComposerAttachment {
  if ('host' in input) {
    return composerRemoteAttachment(input)
  }
  return {
    kind: 'file',
    id: input.id ?? crypto.randomUUID(),
    name: input.name,
    src: input.src,
    phase: input.phase ?? 'ready',
    progress: input.progress,
    snippet: input.snippet,
  }
}

export function composerAttachmentFromFile(file: File): ComposerFileAttachment {
  return {
    kind: 'file',
    id: crypto.randomUUID(),
    name: file.name,
    src: filePreviewUrl(file),
    phase: 'uploading',
    progress: 0,
  }
}

/**
 * Why a file cannot join a message: only an empty one and one over 25 MB are
 * refused. A name already in the message is no reason, since a name says
 * nothing about a file's contents (`composer.md` § Attachments).
 */
export function attachmentFileError(file: File): string | undefined {
  if (!file.size || file.size > ATTACHMENT_MAX_BYTES) {
    return t('attachments.refused', { name: file.name, limit: formatBytes(ATTACHMENT_MAX_BYTES) })
  }
}

export function attachmentsReady(items: readonly ComposerAttachment[]): boolean {
  return items.every((item) => item.kind === 'reference' || item.phase === 'ready')
}

/** A file on a paired device as the backend names it in a message, for a specimen or a test. */
export function encodeRemoteReference(host: string, path: string, deviceId?: string): string {
  const url = new URL('file:///')
  url.pathname = path.startsWith('/') ? path : `/${path}`
  url.searchParams.set('host', host)
  if (deviceId) {
    url.searchParams.set('deviceId', deviceId)
  }
  return url.href
}

/**
 * A file on a paired device as a message names it: a `file:` URL of its path
 * with the device's name and, as the backend writes it, the device's id.
 */
export function decodeRemoteReference(reference: string): {
  host?: string
  deviceId?: string
  path: string
  name: string
} {
  if (reference.startsWith('file:')) {
    try {
      const url = new URL(reference)
      const path = decodeURIComponent(url.pathname)
      const host = url.searchParams.get('host') ?? undefined
      const deviceId = url.searchParams.get('deviceId') ?? undefined
      return {
        host,
        deviceId,
        path,
        name: baseName(path),
      }
    } catch {
      // A malformed file URL is still a reference string; use the leaf name.
    }
  }
  return {
    path: reference,
    name: baseName(reference),
  }
}

/** Why a message cannot go yet, from the phases of the files it uploads; nothing when all are ready. */
export function attachmentSendBlockReason(
  phases: readonly AttachmentPhase[],
): string | undefined {
  if (phases.includes('missing')) {
    return t('attachments.deleteOrAttachAgain')
  }
  if (phases.includes('failed')) {
    return t('attachments.retryOrRemove')
  }
  if (phases.includes('uploading') || phases.includes('attaching')) {
    return t('attachments.waitForUpload')
  }
}

/**
 * The files a message now has, in the order of its capsules, told apart from
 * the ones the composer goes on carrying: a file whose capsule was deleted
 * stops travelling and waits, since an undo can bring the capsule back, and
 * then it travels again. A file that is not this message's to arrange, such
 * as one of a message already being sent, is left where it is.
 */
export function arrangeCapsuleFiles<T extends { id: string }>(
  carried: readonly T[],
  had: readonly string[],
  ids: readonly string[],
  isSending: (item: T) => boolean = () => false,
): { carried: T[]; stopped: T[]; resumed: T[] } {
  const before = new Set(had)
  const now = new Set(ids)
  const message = ids.flatMap((id) => carried.filter((item) => item.id === id))
  const waiting = carried.filter((item) => !now.has(item.id))
  const moving = (item: T) => !isSending(item)
  return {
    carried: [...message, ...waiting],
    stopped: waiting.filter((item) => before.has(item.id) && moving(item)),
    resumed: message.filter((item) => !before.has(item.id) && moving(item)),
  }
}

/** In-flight uploads by attachment id. Removing an attachment cancels its upload, which then reports nothing. */
export class AttachmentUploadQueue {
  readonly #jobs = new Map<string, AbortController>()

  /**
   * Runs `work` and reports its phase and progress through `apply`. Resolves
   * true once the upload is ready and false when it was cancelled; rejects
   * with the work's error when it fails.
   */
  async start(
    id: string,
    work: (signal: AbortSignal, report: (progress: number) => void) => Promise<void>,
    apply: (update: AttachmentUploadUpdate) => void,
  ): Promise<boolean> {
    this.cancel(id)
    const controller = new AbortController()
    this.#jobs.set(id, controller)
    apply({ phase: 'uploading', progress: 0 })
    try {
      await work(controller.signal, (progress) => {
        if (!controller.signal.aborted)
          apply({ phase: 'uploading', progress: clampUnit(progress) })
      })
      if (controller.signal.aborted)
        return false
      apply({ phase: 'ready' })
      return true
    } catch (error) {
      // A cancelled upload is not a failure: whoever removed the file has moved on.
      if (controller.signal.aborted)
        return false
      throw error
    } finally {
      if (this.#jobs.get(id) === controller)
        this.#jobs.delete(id)
    }
  }

  /** Whether an upload of `id` is in flight. */
  has(id: string): boolean {
    return this.#jobs.has(id)
  }

  cancel(id: string): void {
    this.#jobs.get(id)?.abort()
    this.#jobs.delete(id)
  }

  cancelAll(): void {
    for (const id of [...this.#jobs.keys()]) {
      this.cancel(id)
    }
  }
}

export function filePreviewUrl(file: File): string | undefined {
  if (file.type.startsWith('image/')) {
    return URL.createObjectURL(file)
  }
  if (file.type === '' && /\.(png|jpe?g|gif|webp|bmp)$/i.test(file.name)) {
    return URL.createObjectURL(file)
  }
  return undefined
}

export function dataTransferFiles(transfer: DataTransfer): File[] {
  return [...transfer.files]
}
