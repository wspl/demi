import type { UserContentBlock } from '@demicodes/core'
import { ATTACHMENT_SNIPPET_MAX_CHARS, attachmentSnippet, isTextAttachment, sniffModelMediaType } from '@demicodes/core'

/** A failed upload keeps its capsule, which offers Retry; the toast says why it failed. */
export type AttachmentPhase = 'uploading' | 'ready' | 'failed'

/**
 * A local file on its way to the message. Every file reaches the host's
 * working directory on send and the message names its path; media the model
 * reads natively also rides inline (`product.md` § Attachments).
 */
export interface ComposerFileAttachment {
  kind: 'file'
  id: string
  name: string
  src?: string
  phase: AttachmentPhase
  /** 0–1 while `phase` is `uploading`. */
  progress?: number
  /** The opening of a text file, for the tile that shows it as a page. */
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

/** The opening of a text file, by the same rule the transcript's attachment record uses. */
export async function readTextSnippet(file: File): Promise<string | null> {
  if (!isTextAttachment(file.name, file.type)) {
    return null
  }
  const head = await file.slice(0, ATTACHMENT_SNIPPET_MAX_CHARS * 4).text()
  return attachmentSnippet(head)
}

/** Fills `snippet` on a text attachment once the file's opening has been read. */
export async function attachTextSnippet(
  item: ComposerFileAttachment,
  file: File,
): Promise<void> {
  const snippet = await readTextSnippet(file)
  if (snippet) {
    item.snippet = snippet
  }
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

export function attachmentSendBlockReason(
  items: readonly ComposerAttachment[],
): string | undefined {
  if (items.some((item) => item.kind === 'file' && item.phase === 'failed')) {
    return 'Retry or remove the attachments that did not upload'
  }
  if (items.some((item) => item.kind === 'file' && item.phase === 'uploading')) {
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
 * Keeps a message's files to the capsules in its text, in their order.
 *
 * A file whose capsule was deleted is set aside rather than dropped, since
 * undo brings the capsule back and its file belongs with it; a capsule that
 * comes back takes its file from there. What stays aside is released when the
 * message goes or the composer is taken away. A file that is not the message's
 * to arrange, such as one of a message already being sent, is left where it is.
 */
export function arrangeCapsuleFiles<T extends { id: string }>(
  files: readonly T[],
  aside: readonly T[],
  ids: readonly string[],
  isSending: (item: T) => boolean = () => false,
): { files: T[]; aside: T[]; detached: T[]; restored: T[] } {
  const marked = new Set(ids)
  const sending = files.filter((item) => isSending(item))
  const detached = files.filter((item) => !marked.has(item.id) && !isSending(item))
  const restored = aside.filter((item) => marked.has(item.id))
  const byId = new Map([...files, ...restored].map((item) => [item.id, item]))
  const placed = ids.flatMap((id) => {
    const item = byId.get(id)
    return item && !isSending(item) ? [item] : []
  })
  return {
    files: [...sending, ...placed],
    aside: [...aside.filter((item) => !marked.has(item.id)), ...detached],
    detached,
    restored,
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

export interface FileToUserContentOptions {
  signal?: AbortSignal
  onProgress?: (progress: number) => void
}

function abortError(): DOMException {
  return new DOMException('The operation was aborted.', 'AbortError')
}

function readFileBytes(
  file: File,
  options?: FileToUserContentOptions,
): Promise<Uint8Array> {
  if (!options?.signal && !options?.onProgress) {
    return file.arrayBuffer().then((buffer) => new Uint8Array(buffer))
  }
  if (options.signal?.aborted) {
    return Promise.reject(abortError())
  }
  if (typeof FileReader === 'undefined') {
    options.onProgress?.(0)
    return file.arrayBuffer().then((buffer) => {
      options.onProgress?.(1)
      return new Uint8Array(buffer)
    })
  }
  return new Promise((resolve, reject) => {
    const reader = new FileReader()
    const onAbort = () => reader.abort()
    options.signal?.addEventListener('abort', onAbort, { once: true })
    const done = () => options.signal?.removeEventListener('abort', onAbort)
    reader.onprogress = (event) => {
      if (event.lengthComputable && event.total > 0) {
        options.onProgress?.(event.loaded / event.total)
      }
    }
    reader.onload = () => {
      done()
      options.onProgress?.(1)
      resolve(new Uint8Array(reader.result as ArrayBuffer))
    }
    reader.onerror = () => {
      done()
      reject(reader.error ?? new Error('read failed'))
    }
    reader.onabort = () => {
      done()
      reject(abortError())
    }
    reader.readAsArrayBuffer(file)
  })
}

export async function fileToUserContent(
  file: File,
  options?: FileToUserContentOptions,
): Promise<UserContentBlock> {
  const bytes = await readFileBytes(file, options)
  const sniffed = sniffModelMediaType(bytes)
  if (sniffed?.kind === 'image') {
    return {
      type: 'image',
      source: {
        type: 'binary',
        data: bytes,
        mediaType: sniffed.mediaType,
      },
    }
  }
  if (sniffed?.kind === 'video') {
    return {
      type: 'video',
      source: {
        type: 'binary',
        data: bytes,
        mediaType: sniffed.mediaType,
      },
    }
  }
  return {
    type: 'document',
    source: {
      data: bytes,
      mediaType: file.type || 'application/octet-stream',
      fileName: file.name,
    },
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
