import { reactive } from 'vue'
import { createId } from '@demicodes/utils'
import { joinPath, parentPath, relativePath } from './paths'
import { FileBrowserError, type FileBrowserSource } from './types'

/**
 * What a drop or a pick hands over: a file, or a folder with the folders and
 * files inside it, each by its path in the folder, a folder before anything
 * it holds.
 */
export type UploadItem =
  | { kind: 'file'; file: File }
  | {
    kind: 'folder'
    name: string
    directories: readonly string[]
    files: readonly { path: string; file: File }[]
  }

export function uploadItemName(item: UploadItem): string {
  return item.kind === 'file' ? item.file.name : item.name
}

/**
 * What an upload does about what its path holds already: `add` expects
 * nothing there, and a file it finds stops it; `overwrite` writes over files
 * of the same names, a folder going into the folder there; `replace` deletes
 * what is there first.
 */
export type UploadPlacement = 'add' | 'overwrite' | 'replace'

/** An item and how it goes in. */
export interface PlacedUpload {
  item: UploadItem
  placement: UploadPlacement
}

/** An item whose name its directory has: whether it is a folder, and whether what is there is one. */
export interface UploadClash {
  name: string
  isDirectory: boolean
  takenByDirectory: boolean
}

/** The answers to the question an item asks when its directory has its name. */
export type ClashAnswer = 'replace' | 'merge' | 'skip'

/**
 * How an item whose name its directory has goes in, given the answer; null
 * for Skip. Replace writes a file over a file in place and deletes anything
 * else first. Merge takes a folder into the folder there and a file over the
 * file there; only where a folder meets a file does it delete first.
 */
export function clashPlacement(answer: ClashAnswer, isDirectory: boolean, takenByDirectory: boolean): UploadPlacement | null {
  if (answer === 'skip')
    return null
  if (answer === 'merge')
    return isDirectory === takenByDirectory ? 'overwrite' : 'replace'
  return !isDirectory && !takenByDirectory ? 'overwrite' : 'replace'
}

/** One thing an upload asks of its source, in the order it asks. */
export type UploadStep =
  | { kind: 'remove'; path: string; done: boolean }
  | { kind: 'directory'; path: string; done: boolean }
  | { kind: 'file'; path: string; file: File; replace: boolean; done: boolean }

export type UploadFileStep = Extract<UploadStep, { kind: 'file' }>

/** Why a step failed: its path as it reads inside the upload, `''` for the upload's own. */
export interface UploadFailure {
  path: string
  message: string
}

/**
 * Where one upload is: waiting its turn, on its way, landed, or stopped by
 * failures. On its way, `sent` counts the bytes of its files that have gone,
 * and `rate` is how many a second it has moved over the last few seconds,
 * null until it has moved long enough to tell.
 */
export type FileUploadState =
  | { phase: 'waiting' }
  | { phase: 'uploading'; sent: number; rate: number | null }
  | { phase: 'done' }
  | { phase: 'failed'; failures: UploadFailure[] }

/** How far back an upload's rate looks, and how much of that it needs before it says one. */
const RATE_WINDOW_MS = 3000
const RATE_MIN_SPAN_MS = 500

/** One file or folder sent into a directory of a source. */
export interface FileUpload {
  id: string
  kind: UploadItem['kind']
  /** Where it goes, by absolute path: the file, or the folder. */
  path: string
  /** What it takes, in order; a retry goes on with those not done. */
  steps: UploadStep[]
  state: FileUploadState
}

/** What an upload asks of: its files written, its folders made, what it replaces deleted. */
export type UploadSource = Pick<FileBrowserSource, 'upload' | 'createDirectory' | 'remove'>

export function uploadFiles(upload: FileUpload): UploadFileStep[] {
  return upload.steps.filter((step): step is UploadFileStep => step.kind === 'file')
}

/** The bytes of every file an upload sends. */
export function uploadSize(upload: FileUpload): number {
  return uploadFiles(upload).reduce((sum, step) => sum + step.file.size, 0)
}

/**
 * A source's uploads, in the order they were asked for. One is sent at a
 * time and the rest wait, so a large file does not share the link with the
 * others; a folder sends its files one by one. A file of a folder that fails
 * is noted and the folder goes on; a folder that cannot be made, or what it
 * replaces deleted, stops it. A finished upload stays listed until it is
 * dismissed or cleared; a cancelled one goes at once. A tree hears each
 * directory an upload changes, to list it again.
 */
export class FileUploads {
  readonly items: FileUpload[] = reactive([])
  #running: { id: string; controller: AbortController } | null = null
  readonly #changed = new Set<(directory: string) => void>()

  constructor(private readonly source: UploadSource) {}

  /** Queues each item for `directory`, going in as its placement says. */
  add(directory: string, uploads: readonly PlacedUpload[]): void {
    for (const { item, placement } of uploads) {
      const path = joinPath(directory, uploadItemName(item))
      this.items.push({
        id: createId(),
        kind: item.kind,
        path,
        steps: stepsFor(path, item, placement),
        state: { phase: 'waiting' },
      })
    }
    this.#next()
  }

  /** Stops an upload and drops it from the list; what it has done stays done. */
  cancel(id: string): void {
    if (this.#running?.id === id)
      this.#running.controller.abort()
    this.#remove(id)
  }

  /** Sends a failed upload again, after the ones waiting: the steps that did not get done. */
  retry(id: string): void {
    const index = this.items.findIndex((item) => item.id === id)
    if (index < 0 || this.items[index]!.state.phase !== 'failed')
      return
    const [item] = this.items.splice(index, 1)
    item!.state = { phase: 'waiting' }
    this.items.push(item!)
    this.#next()
  }

  /** Drops a finished upload from the list. */
  dismiss(id: string): void {
    const item = this.items.find((entry) => entry.id === id)
    if (item && (item.state.phase === 'done' || item.state.phase === 'failed'))
      this.#remove(id)
  }

  /** Drops every finished upload. */
  clearFinished(): void {
    for (const item of [...this.items]) {
      if (item.state.phase === 'done' || item.state.phase === 'failed')
        this.#remove(item.id)
    }
  }

  /**
   * Calls `listener` with each directory an upload changes: a file landed in
   * it, a folder made or deleted there. The returned function stops it.
   */
  onChanged(listener: (directory: string) => void): () => void {
    this.#changed.add(listener)
    return () => this.#changed.delete(listener)
  }

  #remove(id: string): void {
    const index = this.items.findIndex((item) => item.id === id)
    if (index >= 0)
      this.items.splice(index, 1)
  }

  /** Starts the first waiting upload, unless one is on its way. */
  #next(): void {
    const upload = this.items.find((entry) => entry.state.phase === 'waiting')
    if (this.#running || !upload)
      return
    const controller = new AbortController()
    this.#running = { id: upload.id, controller }
    void this.#run(upload, controller.signal).then((failures) => {
      // A cancelled upload has left the list already.
      if (controller.signal.aborted)
        return
      upload.state = failures.length === 0 ? { phase: 'done' } : { phase: 'failed', failures }
    }).finally(() => {
      this.#running = null
      this.#next()
    })
  }

  /**
   * Takes the upload's steps not done yet, in order, and answers the
   * failures: a file's is noted and the next step goes on; any other stops
   * the upload.
   */
  async #run(upload: FileUpload, signal: AbortSignal): Promise<UploadFailure[]> {
    const failures: UploadFailure[] = []
    let landed = uploadFiles(upload).reduce((sum, step) => sum + (step.done ? step.file.size : 0), 0)
    const pace = recentPace(landed)
    upload.state = { phase: 'uploading', sent: landed, rate: null }
    for (const step of upload.steps) {
      if (step.done)
        continue
      try {
        await this.#take(step, signal, (sent) => {
          if (!signal.aborted)
            upload.state = { phase: 'uploading', sent: landed + sent, rate: pace(landed + sent) }
        })
      } catch (error) {
        // A cancelled upload says nothing more; it has left the list.
        if (signal.aborted)
          return failures
        failures.push({ path: relativePath(upload.path, step.path), message: failureMessage(error) })
        if (step.kind === 'file')
          continue
        break
      }
      step.done = true
      if (step.kind === 'file')
        landed += step.file.size
      for (const listener of this.#changed)
        listener(parentPath(step.path))
    }
    return failures
  }

  async #take(step: UploadStep, signal: AbortSignal, progress: (sent: number) => void): Promise<void> {
    switch (step.kind) {
      case 'remove':
        if (!this.source.remove)
          throw new Error('This host cannot delete what is there.')
        await this.source.remove(step.path, signal)
        return
      case 'directory':
        if (!this.source.createDirectory)
          throw new Error('This host cannot make folders.')
        await this.source.createDirectory(step.path, signal)
        return
      case 'file':
        if (!this.source.upload)
          throw new Error('This host cannot take files.')
        await this.source.upload(step.path, step.file, { replace: step.replace, signal, progress })
    }
  }
}

/**
 * The steps an item takes into `path`: what is there deleted first when it
 * replaces it; a folder made, then the folders inside it, then its files.
 */
function stepsFor(path: string, item: UploadItem, placement: UploadPlacement): UploadStep[] {
  const steps: UploadStep[] = []
  if (placement === 'replace')
    steps.push({ kind: 'remove', path, done: false })
  const replace = placement === 'overwrite'
  if (item.kind === 'file') {
    steps.push({ kind: 'file', path, file: item.file, replace, done: false })
    return steps
  }
  steps.push({ kind: 'directory', path, done: false })
  for (const directory of item.directories)
    steps.push({ kind: 'directory', path: joinPath(path, directory), done: false })
  for (const entry of item.files)
    steps.push({ kind: 'file', path: joinPath(path, entry.path), file: entry.file, replace, done: false })
  return steps
}

/**
 * Bytes a second over the last `RATE_WINDOW_MS`, from the byte counts it is
 * given as they arrive, starting at `start`; null while they span less than
 * `RATE_MIN_SPAN_MS`.
 */
function recentPace(start: number): (sent: number) => number | null {
  const samples = [{ at: performance.now(), sent: start }]
  return (sent) => {
    const at = performance.now()
    samples.push({ at, sent })
    while (samples.length > 2 && at - samples[0]!.at > RATE_WINDOW_MS)
      samples.shift()
    const first = samples[0]!
    const span = at - first.at
    return span < RATE_MIN_SPAN_MS ? null : (sent - first.sent) / (span / 1000)
  }
}

function failureMessage(error: unknown): string {
  if (error instanceof FileBrowserError && error.kind === 'exists')
    return 'A file with this name is there now.'
  return error instanceof Error ? error.message : String(error)
}

const bySource = new WeakMap<UploadSource, FileUploads>()

/** The uploads into `source`, one list for as long as the source lives, whichever view shows them. */
export function uploadsOf(source: UploadSource): FileUploads {
  let uploads = bySource.get(source)
  if (!uploads) {
    uploads = new FileUploads(source)
    bySource.set(source, uploads)
  }
  return uploads
}
