/** A failed upload keeps its capsule, which offers Retry; the toast says why it failed. */
export type AttachmentPhase = 'uploading' | 'ready' | 'failed'

/**
 * A local file on its way to the message. It is uploaded as it is added, and
 * the message names the upload; on send the backend writes the file to the
 * Host (`product.md` § Attachments).
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

export function fileNameFromPath(path: string): string {
  const leaf = path.replace(/\\/g, '/').replace(/\/+$/, '').split('/').pop()
  return leaf || path
}

export interface AttachmentUploadUpdate {
  phase: AttachmentPhase
  progress?: number
}

/**
 * What the backend answered for an upload (`web-api.md` § Uploads and
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

export const ATTACHMENT_MAX_BYTES = 25 * 1024 * 1024

/**
 * A paste this long is a file, not typing: it lands in the composer as
 * `pasted-text.txt`, keeping the draft readable and the text intact.
 */
export const PASTE_AS_FILE_MIN_CHARS = 2000
export const PASTE_AS_FILE_MIN_LINES = 40
const PASTED_TEXT_BASENAME = 'pasted-text'

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

/** The file a long paste becomes; its name steps past the ones already attached. */
export function pastedTextFile(text: string, existingNames: Iterable<string>): File {
  const taken = new Set(existingNames)
  let name = `${PASTED_TEXT_BASENAME}.txt`
  for (let index = 2; taken.has(name); index += 1) {
    name = `${PASTED_TEXT_BASENAME}-${index}.txt`
  }
  return new File([text], name, { type: 'text/plain' })
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
    name: input.name ?? fileNameFromPath(input.path),
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

export function attachmentFileError(
  file: File,
  existingNames: Iterable<string>,
): string | undefined {
  if (!file.size || file.size > ATTACHMENT_MAX_BYTES) {
    return `${file.name}: choose a nonempty file smaller than 25 MB.`
  }
  if ([...existingNames].includes(file.name)) {
    return `${file.name} is already attached.`
  }
}

export function attachmentsReady(items: readonly ComposerAttachment[]): boolean {
  return items.every((item) => item.kind === 'reference' || item.phase === 'ready')
}

export function encodeRemoteReference(host: string, path: string): string {
  const url = new URL('file:///')
  url.pathname = path.startsWith('/') ? path : `/${path}`
  url.searchParams.set('host', host)
  return url.href
}

export function decodeRemoteReference(reference: string): {
  host?: string
  path: string
  name: string
} {
  if (reference.startsWith('file:')) {
    try {
      const url = new URL(reference)
      const path = decodeURIComponent(url.pathname)
      const host = url.searchParams.get('host') ?? undefined
      return {
        host,
        path,
        name: fileNameFromPath(path),
      }
    } catch {
      // A malformed file URL is still a reference string; use the leaf name.
    }
  }
  return {
    path: reference,
    name: fileNameFromPath(reference),
  }
}

/** Why a message cannot go yet, from the phases of the files it uploads; nothing when all are ready. */
export function attachmentSendBlockReason(
  phases: readonly AttachmentPhase[],
): string | undefined {
  if (phases.includes('failed')) {
    return 'Retry or remove the attachments that did not upload'
  }
  if (phases.includes('uploading')) {
    return 'Wait for attachments to finish uploading'
  }
}

export function remoteAttachmentError(
  path: string,
  host: string,
  existing: readonly ComposerAttachment[],
): string | undefined {
  if (
    existing.some(
      (item) =>
        item.kind === 'reference' && item.path === path && item.host === host,
    )
  ) {
    return `${fileNameFromPath(path)} is already attached.`
  }
}

export function composerFileNames(items: readonly ComposerAttachment[]): string[] {
  return items.filter(isComposerFile).map((item) => item.name)
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
