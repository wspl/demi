/**
 * What a page keeps of one Host's files (`plugin-pages.md` § What the
 * service keeps): listings, texts, descriptions, the changes list and each
 * changed file's sides, each the last answer read, by the absolute path on
 * the Host. An entry read while a live watch covered its path is confirmed:
 * showing it again asks nothing until a report of that watch names its path.
 * Any other entry is unconfirmed: showing it shows it at once and reads it
 * again. Reads of one entry share one request, and a report that names an
 * entry being read reads it once more after. At most `budget` of text stay,
 * the entries shown longest ago leaving first.
 */
import { reactive } from 'vue'
import { parentPath } from './paths'
import { FileBrowserError, type FileBrowserFailure } from './types'

/** The most text a page keeps of the Hosts' files, counted in characters. */
export const KEPT_TEXT = 64 * 1024 * 1024

/**
 * What an entry is, which decides which reports concern it: a listing is
 * its folder's, and a file's entries are the file's; the changes list and
 * the committed sides follow the repository's `.git` too.
 */
export type KeptKind = 'listing' | 'text' | 'description' | 'changes' | 'sides' | 'committed'

/** One kept answer as a view shows it; reactive. */
export interface ShownEntry<T> {
  /** The last answer read; undefined before the first, and once a read found the path gone. */
  readonly value: T | undefined
  /**
   * Why the last read failed. Without a value it is what the view shows, as
   * a first read would; beside a value the view keeps the value and says
   * quietly that it could not refresh.
   */
  readonly failure: FileBrowserFailure | null
  /** A read is on its way: the first, which a view shows as loading, or a check, which it does not. */
  readonly reading: boolean
}

/** A view's hold on an entry it shows. */
export interface Showing<T> {
  readonly entry: ShownEntry<T>
  /** Reads the entry again now, as Retry does. */
  retry(): void
  /** The view no longer shows it; a second release does nothing. */
  release(): void
}

/** What a live watch of the Host covers: a report of it names every change at such a path. */
export interface Coverage {
  covers(path: string): boolean
}

/** How an entry is read and measured. */
export interface KeptSpec<T> {
  kind: KeptKind
  /** The absolute path on the Host whose reports concern it: a listing's folder, a file, the changes list's root. */
  path: string
  /** Tells the entries of one path and kind apart, such as a changed file's sides from its working tree root. */
  key?: string
  /** Reads it; `held` is the last answer kept, which a read may answer unchanged with. */
  read(held: T | undefined): Promise<T>
  /** The characters of text it holds. */
  size(value: T): number
}

/** Whether `path` lies in a repository's `.git`, whose changes concern the changes list and the committed sides. */
export function inGitDirectory(path: string): boolean {
  return /(^|\/)\.git(\/|$)/.test(path.replaceAll('\\', '/'))
}

/** Whether `path` is `ancestor` or lies below it. */
function within(path: string, ancestor: string): boolean {
  const prefix = ancestor.endsWith('/') ? ancestor : `${ancestor}/`
  return path === ancestor || path.startsWith(prefix)
}

class Entry<T> {
  readonly state: ShownEntry<T> & { value: T | undefined; failure: FileBrowserFailure | null; reading: boolean }
  confirmed = false
  /** How many views show it. */
  shown = 0
  /** When it was last shown, by the cache's clock. */
  lastShown = 0
  size = 0
  /** Rises with each report that names it: a read that began before one cannot confirm it. */
  stamp = 0
  reading: Promise<void> | null = null

  constructor(readonly spec: KeptSpec<T>) {
    this.state = reactive({ value: undefined, failure: null, reading: false }) as Entry<T>['state']
  }

  /** Whether a report of a change at `path` concerns it. */
  concerns(path: string): boolean {
    const own = this.spec.path
    switch (this.spec.kind) {
      case 'listing':
        // Its own folder, a folder above it, or an entry of it came, went or was renamed.
        return within(own, path) || parentPath(path) === own
      case 'text':
      case 'description':
        return within(own, path)
      case 'sides':
        return within(own, path) || inGitDirectory(path)
      case 'changes':
        return within(path, own) || inGitDirectory(path)
      case 'committed':
        return inGitDirectory(path)
    }
  }
}

function failureOf(error: unknown): FileBrowserFailure {
  if (error instanceof FileBrowserError)
    return { kind: error.kind, message: error.message === error.kind ? undefined : error.message }
  return { kind: 'other', message: error instanceof Error ? error.message : String(error) }
}

/** The kept files of one Host, which every conversation and page on it shares. */
export class HostFiles {
  private readonly entries = new Map<string, Entry<unknown>>()
  private readonly coverages = new Set<Coverage>()
  private clock = 0
  private kept = 0

  constructor(private readonly budget = KEPT_TEXT) {}

  private entry<T>(spec: KeptSpec<T>): Entry<T> {
    const key = `${spec.kind}\u0000${spec.path}\u0000${spec.key ?? ''}`
    let entry = this.entries.get(key) as Entry<T> | undefined
    if (!entry) {
      entry = new Entry(spec)
      this.entries.set(key, entry as Entry<unknown>)
    }
    return entry
  }

  /** The live coverage of `path`, if any. */
  private coverageOf(path: string): Coverage | null {
    for (const coverage of this.coverages) {
      if (coverage.covers(path))
        return coverage
    }
    return null
  }

  /**
   * Reads `entry`, or joins the read on its way. The answer confirms it
   * when a live watch covered its path from the read's start to its end and
   * no report named it meanwhile; a report that did reads it once more for
   * the views that show it.
   */
  private read<T>(entry: Entry<T>): Promise<void> {
    if (entry.reading)
      return entry.reading
    const stamp = entry.stamp
    const coverage = this.coverageOf(entry.spec.path)
    entry.state.reading = true
    const reading = (async () => {
      try {
        const value = await entry.spec.read(entry.state.value)
        entry.state.value = value
        entry.state.failure = null
        this.measure(entry, entry.spec.size(value))
        entry.confirmed = this.confirms(coverage, entry, stamp)
      } catch (error) {
        const failure = failureOf(error)
        entry.state.failure = failure
        // A path that is gone is an answer, as a first read's would be; any
        // other failure keeps what the view shows.
        if (failure.kind === 'not-found') {
          entry.state.value = undefined
          this.measure(entry, 0)
          entry.confirmed = this.confirms(coverage, entry, stamp)
        } else {
          entry.confirmed = false
        }
      } finally {
        entry.reading = null
        entry.state.reading = false
      }
      if (entry.stamp !== stamp && entry.shown > 0)
        void this.read(entry)
      this.evict()
    })()
    entry.reading = reading
    return reading
  }

  private confirms(coverage: Coverage | null, entry: Entry<unknown>, stamp: number): boolean {
    return coverage !== null && this.coverages.has(coverage) && entry.stamp === stamp
  }

  private measure(entry: Entry<unknown>, size: number): void {
    this.kept += size - entry.size
    entry.size = size
  }

  /** Lets go of the entries shown longest ago, none shown now or being read, until the text kept fits the budget. */
  private evict(): void {
    if (this.kept <= this.budget)
      return
    const idle = [...this.entries.entries()]
      .filter(([, entry]) => entry.shown === 0 && !entry.reading)
      .sort(([, a], [, b]) => a.lastShown - b.lastShown)
    for (const [key, entry] of idle) {
      if (this.kept <= this.budget)
        return
      this.kept -= entry.size
      this.entries.delete(key)
    }
  }

  private unconfirm(entry: Entry<unknown>): void {
    entry.confirmed = false
    entry.stamp += 1
  }

  /**
   * A view shows the entry `spec` names until it releases it: what is kept
   * shows at once, and an entry not kept or not confirmed is read.
   */
  show<T>(spec: KeptSpec<T>): Showing<T> {
    const entry = this.entry(spec)
    entry.shown += 1
    entry.lastShown = ++this.clock
    if (!entry.confirmed)
      void this.read(entry)
    let released = false
    return {
      entry: entry.state,
      retry: () => {
        void this.read(entry)
      },
      release: () => {
        if (released)
          return
        released = true
        entry.shown -= 1
        entry.lastShown = ++this.clock
        this.evict()
      },
    }
  }

  /** Reads the entry `spec` names again now, as a Refresh does. */
  retry<T>(spec: KeptSpec<T>): void {
    void this.read(this.entry(spec))
  }

  /** What is kept of the entry `spec` names, as a view would show it, without showing or reading it. */
  peek<T>(spec: KeptSpec<T>): ShownEntry<T> {
    return this.entry(spec).state
  }

  /** The entry once, for work that shows nothing: the confirmed answer kept, or a read's. */
  async get<T>(spec: KeptSpec<T>): Promise<T> {
    const entry = this.entry(spec)
    entry.lastShown = ++this.clock
    if (!entry.confirmed)
      await this.read(entry)
    const { value, failure } = entry.state
    if (value === undefined)
      throw new FileBrowserError(failure?.kind ?? 'other', failure?.message)
    return value
  }

  /**
   * Something changed at each of `paths`: every entry they concern is
   * unconfirmed, and read again at once while a view shows it.
   */
  changed(paths: readonly string[]): void {
    for (const entry of this.entries.values()) {
      if (!paths.some((path) => entry.concerns(path)))
        continue
      this.unconfirm(entry)
      if (entry.shown > 0)
        void this.read(entry)
    }
  }

  /** A watch is live: what it covers, read from now on, is confirmed. */
  cover(coverage: Coverage): void {
    this.coverages.add(coverage)
  }

  /**
   * A watch stopped covering: it ended, or lost reports. What it covered
   * may have changed unreported, so it is unconfirmed; with `reread`, as
   * after lost reports on a watch that goes on, the entries shown are read
   * again at once.
   */
  uncover(coverage: Coverage, reread = false): void {
    if (!this.coverages.delete(coverage))
      return
    for (const entry of this.entries.values()) {
      if (!coverage.covers(entry.spec.path))
        continue
      this.unconfirm(entry)
      if (reread && entry.shown > 0)
        void this.read(entry)
    }
  }

  /** Reads every shown entry `where` picks again now, as a Refresh does. */
  refresh(where: (path: string) => boolean = () => true): void {
    for (const entry of this.entries.values()) {
      if (entry.shown > 0 && where(entry.spec.path))
        void this.read(entry)
    }
  }
}
