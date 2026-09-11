import type { UserContentBlock } from '@demicodes/core'
import { fileExtensionSupport, sniffModelMediaType } from '@demicodes/core'
import { delay } from '@demicodes/utils'
import { t } from '../../infra/i18n'

export type AttachmentDestination = 'message' | 'workspace'
export type AttachmentPhase = 'staged' | 'uploading' | 'ready' | 'failed'

/** A local file: one phase, and whether it goes to the model or the working directory. */
export interface ComposerFileAttachment {
  kind: 'file'
  id: string
  name: string
  src?: string
  destination: AttachmentDestination
  phase: AttachmentPhase
  error?: string
  /** 0–1 while `phase` is `uploading`. */
  progress?: number
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

export function isComposerRemote(
  item: ComposerAttachment,
): item is ComposerRemoteAttachment {
  return item.kind === 'reference'
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
export const PROTOTYPE_UPLOAD_MS = 1400

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

export function attachmentDestination(
  file: File,
  acceptedExtensions: readonly string[] | null,
): AttachmentDestination {
  return fileMatchesAcceptedExtensions(file, acceptedExtensions)
    ? 'message'
    : 'workspace'
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
    destination: input.destination ?? 'workspace',
    phase: input.phase ?? 'ready',
    error: input.error,
    progress: input.progress,
  }
}

export function composerAttachmentFromFile(
  file: File,
  acceptedExtensions: readonly string[] | null,
): ComposerFileAttachment {
  return {
    kind: 'file',
    id: crypto.randomUUID(),
    name: file.name,
    src: filePreviewUrl(file),
    destination: attachmentDestination(file, acceptedExtensions),
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
  return items.every((item) => item.kind === 'reference' || item.phase === 'ready' || item.phase === 'staged')
}

export function attachmentCaption(item: ComposerAttachment): string {
  if (item.kind === 'file' && item.phase === 'staged') {
    return `Uploads when you send · ${item.name}`
  }
  if (item.kind === 'reference') {
    return `${item.host} · ${item.path}`
  }
  if (item.phase === 'failed') {
    return `${item.error ?? 'Upload failed'} · ${item.name}`
  }
  if (item.phase === 'uploading') {
    return `${t('agent.input.attachmentUploading')} ${Math.round(attachmentProgress(item) * 100)}% · ${item.name}`
  }
  const destination =
    item.destination === 'message'
      ? t('agent.input.attachmentMessage')
      : t('agent.input.attachmentWorkspace')
  return `${destination} · ${item.name}`
}

/** The failed uploads, one line each, for the error under the composer; null when none failed. */
export function attachmentFailureText(items: readonly ComposerAttachment[]): string | null {
  const lines = items
    .filter((item) => item.kind === 'file' && item.phase === 'failed')
    .map((item) => `${item.name}: ${item.kind === 'file' && item.error ? item.error : 'Upload failed'}`)
  return lines.length > 0 ? lines.join('\n') : null
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

export function contentBlockCaption(block: UserContentBlock): string | undefined {
  if (block.type === 'image') {
    const name = imageNameFromSource(block.source)
    return `${t('agent.input.attachmentMessage')} · ${name}`
  }
  if (block.type === 'document') {
    return `${t('agent.input.attachmentMessage')} · ${block.source.fileName}`
  }
  if (block.type === 'reference') {
    const { host, path } = decodeRemoteReference(block.reference)
    return host ? `${host} · ${path}` : path
  }
}

function imageNameFromSource(
  source: Extract<UserContentBlock, { type: 'image' }>['source'],
): string {
  if (source.type === 'url') {
    const leaf = source.url.split('/').pop()
    return leaf ? decodeURIComponent(leaf) : 'image'
  }
  return 'image'
}

export function attachmentSendBlockReason(
  items: readonly ComposerAttachment[],
): string | undefined {
  if (items.some((item) => item.kind === 'file' && item.phase === 'uploading')) {
    return t('agent.input.waitForAttachments')
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

/** In-flight upload jobs. Cancel on remove; a finished job is a no-op if the file is gone. */
export class AttachmentUploadQueue {
  readonly #jobs = new Map<string, AbortController>()

  start(
    id: string,
    work: (signal: AbortSignal, report: (progress: number) => void) => Promise<void>,
    apply: (update: AttachmentUploadUpdate) => void,
    onFail?: (id: string) => void,
  ): void {
    this.cancel(id)
    const ac = new AbortController()
    this.#jobs.set(id, ac)
    apply({
      phase: 'uploading',
      progress: 0,
    })
    const report = (progress: number) => {
      if (!ac.signal.aborted) {
        apply({
          phase: 'uploading',
          progress: clampUnit(progress),
        })
      }
    }
    void work(ac.signal, report)
      .then(() => {
        if (!ac.signal.aborted) {
          apply({ phase: 'ready' })
        }
      })
      .catch(() => {
        if (!ac.signal.aborted) {
          onFail?.(id)
        }
      })
      .finally(() => {
        if (this.#jobs.get(id) === ac) {
          this.#jobs.delete(id)
        }
      })
  }

  /** Prototype host: a timed sweep from 0 to 1, then ready (or `onFail` when `fail` is set). */
  startPrototype(
    id: string,
    apply: (update: AttachmentUploadUpdate) => void,
    onFail?: (id: string) => void,
    fail = false,
  ): void {
    this.start(
      id,
      (signal, report) => runPrototypeProgress(signal, report, fail),
      apply,
      onFail,
    )
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

async function runPrototypeProgress(
  signal: AbortSignal,
  report: (progress: number) => void,
  fail: boolean,
): Promise<void> {
  const started = performance.now()
  report(0)
  while (!signal.aborted) {
    const progress = clampUnit((performance.now() - started) / PROTOTYPE_UPLOAD_MS)
    report(progress)
    if (progress >= 1) {
      if (fail) {
        throw new Error('upload failed')
      }
      return
    }
    await delay(16, signal)
  }
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

export function fileMatchesAcceptedExtensions(
  file: File,
  acceptedExtensions: readonly string[] | null,
): boolean {
  const ext = file.name.split('.').pop()?.toLowerCase()
  if (!ext) {
    return false
  }
  return fileExtensionSupport(acceptedExtensions, ext) === true
}

/** Splits a drop or paste into the files the model accepts and the ones it does not. */
export function partitionAcceptedFiles(
  files: readonly File[],
  acceptedExtensions: readonly string[] | null,
): {
  accepted: File[]
  rejected: File[]
} {
  const accepted: File[] = []
  const rejected: File[] = []
  for (const file of files) {
    ;(fileMatchesAcceptedExtensions(file, acceptedExtensions)
      ? accepted
      : rejected
    ).push(file)
  }
  return {
    accepted,
    rejected,
  }
}

export function acceptAttribute(
  acceptedExtensions: readonly string[] | null,
): string | undefined {
  if (acceptedExtensions === null || acceptedExtensions.length === 0) {
    return undefined
  }
  return acceptedExtensions.map((ext) => `.${ext}`).join(',')
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

export function transferHasFiles(
  transfer: DataTransfer | null | undefined,
): boolean {
  return transfer?.types.includes('Files') === true
}
