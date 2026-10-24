import { shallowRef, triggerRef, type ShallowRef } from 'vue'
import { compareFileNames } from './file-browser-state'
import type { FileBrowserSource, FileSearchMatch, FileSearchQuery } from './types'

/** A file a search found, by its path relative to the directory searched, with its matches. */
export interface FoundFile {
  path: string
  matches: FileSearchMatch[]
}

/** How a search stands: none run yet, running, ended with what it found, or failed. */
export type SearchState =
  | { kind: 'idle' }
  | { kind: 'searching' }
  | { kind: 'done', files: number, matches: number, limitReached: boolean }
  | { kind: 'failed', error: unknown }

/**
 * Paths in the order the tree lists them: part by part, a folder before the
 * files beside it, names as the tree compares them.
 */
export function compareFoundPaths(a: string, b: string): number {
  const left = a.split('/')
  const right = b.split('/')
  for (let index = 0; index < Math.min(left.length, right.length); index++) {
    const leftFolder = index < left.length - 1
    const rightFolder = index < right.length - 1
    if (leftFolder !== rightFolder)
      return leftFolder ? -1 : 1
    const order = compareFileNames(left[index]!, right[index]!)
    if (order !== 0)
      return order
  }
  return left.length - right.length
}

/** The globs a field holds, separated by commas as VS Code's are. */
export function globsOf(text: string): string[] {
  return text.split(',').map((glob) => glob.trim()).filter((glob) => glob !== '')
}

/**
 * A search of a source's files, one at a time: a new one stops the one that
 * runs. The files found come in the order the Host finds them and are kept in
 * the tree's order, shown a frame at a time so a search that finds thousands
 * does not draw thousands of times.
 */
export class TextSearch {
  readonly files: ShallowRef<FoundFile[]> = shallowRef([])
  readonly state: ShallowRef<SearchState> = shallowRef({ kind: 'idle' })
  private running: AbortController | null = null
  private drawing = 0

  /** Searches `root` for `query`, ending the search before it. */
  async run(source: FileBrowserSource, root: string, query: FileSearchQuery): Promise<void> {
    this.stop()
    const search = source.search
    if (!search)
      return
    const controller = new AbortController()
    this.running = controller
    this.files.value = []
    this.state.value = { kind: 'searching' }
    try {
      for await (const event of search(root, query, controller.signal)) {
        if (controller.signal.aborted)
          return
        if (event.type === 'file') {
          this.insert({ path: event.path, matches: event.matches })
          continue
        }
        this.draw()
        this.state.value = { kind: 'done', files: event.files, matches: event.matches, limitReached: event.limitReached }
      }
    } catch (error) {
      if (controller.signal.aborted)
        return
      this.draw()
      this.state.value = { kind: 'failed', error }
    } finally {
      if (this.running === controller)
        this.running = null
    }
  }

  /** Stops the search that runs, keeping what it found. */
  stop(): void {
    this.running?.abort()
    this.running = null
    if (this.state.value.kind === 'searching')
      this.state.value = { kind: 'idle' }
  }

  /** Forgets what the last search found. */
  clear(): void {
    this.stop()
    this.files.value = []
    this.state.value = { kind: 'idle' }
  }

  private insert(file: FoundFile): void {
    const files = this.files.value
    let low = 0
    let high = files.length
    while (low < high) {
      const middle = (low + high) >> 1
      if (compareFoundPaths(files[middle]!.path, file.path) < 0)
        low = middle + 1
      else
        high = middle
    }
    files.splice(low, 0, file)
    if (this.drawing === 0)
      this.drawing = requestAnimationFrame(() => this.draw())
  }

  private draw(): void {
    if (this.drawing !== 0)
      cancelAnimationFrame(this.drawing)
    this.drawing = 0
    triggerRef(this.files)
  }
}
