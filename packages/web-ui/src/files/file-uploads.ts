import { reactive } from 'vue'
import { createId } from '@demicodes/utils'
import { joinPath, parentPath } from './paths'
import { FileBrowserError, type FileBrowserSource } from './types'

/**
 * Where one upload is: waiting its turn, on its way, landed, or stopped by a
 * failure. On its way, `rate` is how many bytes a second it has moved over
 * the last few seconds, null until it has moved long enough to tell.
 */
export type FileUploadState =
  | { phase: 'waiting' }
  | { phase: 'uploading'; sent: number; rate: number | null }
  | { phase: 'done' }
  | { phase: 'failed'; message: string }

/** How far back an upload's rate looks, and how much of that it needs before it says one. */
const RATE_WINDOW_MS = 3000
const RATE_MIN_SPAN_MS = 500

/** One file sent into a directory of a source. */
export interface FileUpload {
  id: string
  /** Where the file goes, by absolute path. */
  path: string
  file: File
  /** Whether it may replace a file already at `path`. */
  replace: boolean
  state: FileUploadState
}

/**
 * A source's uploads, in the order they were asked for. One is sent at a
 * time and the rest wait, so a large file does not share the link with the
 * others. A finished upload stays listed until it is dismissed or cleared; a
 * cancelled one goes at once. A tree hears each upload that lands, to list its
 * directory again.
 */
export class FileUploads {
  readonly items: FileUpload[] = reactive([])
  #running: { id: string; controller: AbortController } | null = null
  readonly #landed = new Set<(directory: string) => void>()

  constructor(private readonly source: Pick<FileBrowserSource, 'upload'>) {}

  /** Queues `files` for `directory`; a name in `replace` may replace the file already there. */
  add(directory: string, files: readonly File[], replace: ReadonlySet<string>): void {
    for (const file of files) {
      this.items.push({
        id: createId(),
        path: joinPath(directory, file.name),
        file,
        replace: replace.has(file.name),
        state: { phase: 'waiting' },
      })
    }
    this.#next()
  }

  /** Stops an upload and drops it from the list; a stopped upload leaves its path as it was. */
  cancel(id: string): void {
    if (this.#running?.id === id)
      this.#running.controller.abort()
    this.#remove(id)
  }

  /** Sends a failed upload again, after the ones waiting. */
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

  /** Calls `listener` with the directory of each upload that lands; the returned function stops it. */
  onLanded(listener: (directory: string) => void): () => void {
    this.#landed.add(listener)
    return () => this.#landed.delete(listener)
  }

  #remove(id: string): void {
    const index = this.items.findIndex((item) => item.id === id)
    if (index >= 0)
      this.items.splice(index, 1)
  }

  /** Starts the first waiting upload, unless one is on its way. */
  #next(): void {
    const item = this.items.find((entry) => entry.state.phase === 'waiting')
    if (this.#running || !this.source.upload || !item)
      return
    const controller = new AbortController()
    this.#running = { id: item.id, controller }
    item.state = { phase: 'uploading', sent: 0, rate: null }
    const pace = recentPace()
    void this.source.upload(item.path, item.file, {
      replace: item.replace,
      signal: controller.signal,
      progress: (sent) => {
        if (!controller.signal.aborted)
          item.state = { phase: 'uploading', sent, rate: pace(sent) }
      },
    }).then(
      () => {
        if (controller.signal.aborted)
          return
        item.state = { phase: 'done' }
        for (const listener of this.#landed)
          listener(parentPath(item.path))
      },
      (error: unknown) => {
        // A cancelled upload has left the list already; its rejection says nothing new.
        if (controller.signal.aborted)
          return
        item.state = { phase: 'failed', message: failureMessage(error) }
      },
    ).finally(() => {
      this.#running = null
      this.#next()
    })
  }
}

/**
 * Bytes a second over the last `RATE_WINDOW_MS`, from the byte counts it is
 * given as they arrive; null while they span less than `RATE_MIN_SPAN_MS`.
 */
function recentPace(): (sent: number) => number | null {
  const samples = [{ at: performance.now(), sent: 0 }]
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

const bySource = new WeakMap<Pick<FileBrowserSource, 'upload'>, FileUploads>()

/** The uploads into `source`, one list for as long as the source lives, whichever view shows them. */
export function uploadsOf(source: Pick<FileBrowserSource, 'upload'>): FileUploads {
  let uploads = bySource.get(source)
  if (!uploads) {
    uploads = new FileUploads(source)
    bySource.set(source, uploads)
  }
  return uploads
}
