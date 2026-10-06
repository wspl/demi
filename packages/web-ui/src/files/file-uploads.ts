import { reactive } from 'vue'
import { createId } from '@demicodes/utils'
import { joinPath, relativePath } from './paths'
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

/**
 * One thing an upload asks of its source: what it replaces deleted first,
 * then its files written, whose writes make the folders above them, and the
 * folders no file makes, the empty ones.
 */
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

/** How many steps run at once, across every upload of a source (`web-application.md` § Requests for one action). */
export const UPLOADS_AT_ONCE = 4

/** One file or folder sent into a directory of a source. */
export interface FileUpload {
  id: string
  kind: UploadItem['kind']
  /** Where it goes, by absolute path: the file, or the folder. */
  path: string
  /** What it takes; a retry goes on with those not done. */
  steps: UploadStep[]
  state: FileUploadState
}

/** What an upload asks of: its files written, its empty folders made, what it replaces deleted. */
export type UploadSource = Pick<FileBrowserSource, 'upload' | 'createDirectory' | 'remove'>

export function uploadFiles(upload: FileUpload): UploadFileStep[] {
  return upload.steps.filter((step): step is UploadFileStep => step.kind === 'file')
}

/** The bytes of every file an upload sends. */
export function uploadSize(upload: FileUpload): number {
  return uploadFiles(upload).reduce((sum, step) => sum + step.file.size, 0)
}

/**
 * An upload under way: what stops it, the steps it has running with the
 * bytes each has sent, and the steps that failed with why.
 */
interface Run {
  controller: AbortController
  running: Map<UploadStep, number>
  failed: Map<UploadStep, string>
  /** A step other than a file's failed: nothing more starts. */
  stopped: boolean
  pace: (sent: number) => number | null
}

/**
 * A source's uploads, in the order they were asked for. Up to
 * `UPLOADS_AT_ONCE` steps run at once, taken in that order: a large file
 * does not hold the ones behind it, and a folder's files go several at a
 * time. An upload that replaces something deletes it before anything else
 * of it starts. A file of a folder that fails is noted and the folder goes
 * on; a folder that cannot be made, or what it replaces deleted, stops it.
 * A finished upload stays listed until it is dismissed or cleared; a
 * cancelled one goes at once. What each answer changed, the source's write
 * marks, so the views that show it read it again (`keptSource`).
 */
export class FileUploads {
  readonly items: FileUpload[] = reactive([])
  readonly #runs = new Map<string, Run>()
  #active = 0

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
    this.#pump()
  }

  /** Stops an upload and drops it from the list; what it has done stays done. */
  cancel(id: string): void {
    this.#runs.get(id)?.controller.abort()
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
    this.#pump()
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

  #remove(id: string): void {
    const index = this.items.findIndex((item) => item.id === id)
    if (index >= 0)
      this.items.splice(index, 1)
  }

  /** Starts the steps next in line while fewer than `UPLOADS_AT_ONCE` run. */
  #pump(): void {
    while (this.#active < UPLOADS_AT_ONCE) {
      const next = this.#nextStep()
      if (!next)
        return
      this.#start(next.upload, next.step)
    }
  }

  /** The first step not done and not running of the first upload under way that can start one. */
  #nextStep(): { upload: FileUpload; step: UploadStep } | null {
    for (const upload of this.items) {
      if (upload.state.phase !== 'waiting' && upload.state.phase !== 'uploading')
        continue
      const run = this.#runs.get(upload.id)
      if (run?.stopped)
        continue
      const running = run?.running ?? new Map<UploadStep, number>()
      // What the upload replaces goes before anything else of it starts.
      const removal = upload.steps.find((step) => step.kind === 'remove' && !step.done)
      if (removal) {
        if (running.size === 0)
          return { upload, step: removal }
        continue
      }
      const step = upload.steps.find((candidate) => !candidate.done && !running.has(candidate) && !run?.failed.has(candidate))
      if (step)
        return { upload, step }
    }
    return null
  }

  #start(upload: FileUpload, step: UploadStep): void {
    let run = this.#runs.get(upload.id)
    if (!run) {
      run = {
        controller: new AbortController(),
        running: new Map(),
        failed: new Map(),
        stopped: false,
        pace: recentPace(landedBytes(upload)),
      }
      this.#runs.set(upload.id, run)
      upload.state = { phase: 'uploading', sent: landedBytes(upload), rate: null }
    }
    const current = run
    const { signal } = current.controller
    current.running.set(step, 0)
    this.#active += 1
    void this.#take(step, signal, (sent) => {
      if (signal.aborted)
        return
      current.running.set(step, sent)
      report(upload, current)
    }).then(() => {
      step.done = true
      current.running.delete(step)
      report(upload, current)
    }, (error: unknown) => {
      // A cancelled upload says nothing more; it has left the list.
      if (signal.aborted)
        return
      current.failed.set(step, failureMessage(error))
      if (step.kind !== 'file')
        current.stopped = true
    }).finally(() => {
      current.running.delete(step)
      this.#active -= 1
      if (!signal.aborted)
        this.#settle(upload, current)
      else if (current.running.size === 0)
        this.#runs.delete(upload.id)
      this.#pump()
    })
  }

  /** Ends an upload that has nothing more to run: done, or failed with what failed. */
  #settle(upload: FileUpload, run: Run): void {
    if (run.running.size > 0)
      return
    const more = !run.stopped && upload.steps.some((step) => !step.done && !run.failed.has(step))
    if (more)
      return
    this.#runs.delete(upload.id)
    const failures = upload.steps.flatMap((step) => {
      const message = run.failed.get(step)
      return message === undefined ? [] : [{ path: relativePath(upload.path, step.path), message }]
    })
    upload.state = failures.length === 0 ? { phase: 'done' } : { phase: 'failed', failures }
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

/** How far an upload on its way is: its files landed and what its running ones have sent. */
function report(upload: FileUpload, run: Run): void {
  const sent = landedBytes(upload) + [...run.running.values()].reduce((sum, bytes) => sum + bytes, 0)
  upload.state = { phase: 'uploading', sent, rate: run.pace(sent) }
}

/** The bytes of an upload's files that have landed. */
function landedBytes(upload: FileUpload): number {
  return uploadFiles(upload).reduce((sum, step) => sum + (step.done ? step.file.size : 0), 0)
}

/**
 * The steps an item takes into `path`: what is there deleted first when it
 * replaces it; then its files, whose writes make the folders above them, and
 * the folders that hold no file, which only a folder's own step makes.
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
  for (const entry of item.files)
    steps.push({ kind: 'file', path: joinPath(path, entry.path), file: entry.file, replace, done: false })
  // A folder another one or a file is in is made on the way to it.
  const inside = (folder: string, other: string) => other.startsWith(`${folder}/`)
  const holders = [...item.directories, ...item.files.map((entry) => entry.path)]
  const empty = item.directories.filter((folder) => !holders.some((other) => inside(folder, other)))
  for (const folder of empty)
    steps.push({ kind: 'directory', path: joinPath(path, folder), done: false })
  if (holders.length === 0)
    steps.push({ kind: 'directory', path, done: false })
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
